//! Explicit resolution policy for occurrence-aware graph consumers.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{
    node_resolver::{
        caching::NodeModulesResolverOptions, combined_resolver::CombinedResolverCaches,
    },
    resolve::{PathResolver, TargetEnv, NODE_BUILTINS},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMode {
    Import,
    Require,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDependency {
    File(PathBuf),
    External(String),
}

pub trait DependencyResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency>;
}

/// No implicit externals: exact raw specifiers must be explicitly allowed.
pub struct StructuralResolver {
    root: PathBuf,
    options: NodeModulesResolverOptions,
    externals: BTreeSet<String>,
    import_cache: CombinedResolverCaches,
    require_cache: CombinedResolverCaches,
    revision: u64,
}

impl StructuralResolver {
    pub fn new(
        root: PathBuf,
        options: NodeModulesResolverOptions,
        externals: BTreeSet<String>,
    ) -> Self {
        Self {
            root,
            options,
            externals,
            import_cache: CombinedResolverCaches::new(),
            require_cache: CombinedResolverCaches::new(),
            revision: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Coarse invalidation is intentional: misses and configuration ancestry also change.
    pub fn invalidate(&mut self) {
        self.import_cache = CombinedResolverCaches::new();
        self.require_cache = CombinedResolverCaches::new();
        self.revision += 1;
    }

    pub fn set_policy(&mut self, options: NodeModulesResolverOptions, externals: BTreeSet<String>) {
        self.options = options;
        self.externals = externals;
        self.invalidate();
    }
}

impl DependencyResolver for StructuralResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        // Check the same one-step exact replacement used by the file resolver.
        let replacement = self.options.alias.get(specifier);
        let target = replacement.map(String::as_str).unwrap_or(specifier);
        eyre::ensure!(
            self.options.target_env != TargetEnv::Browser
                || (!target.starts_with("node:") && !NODE_BUILTINS.contains(&target)),
            "{mode:?} resolving {specifier:?} from {}: Node builtin {target:?} is unsupported for the browser target; no polyfills are provided",
            base.display()
        );
        if replacement.is_none() && self.externals.contains(specifier) {
            return Ok(ResolvedDependency::External(specifier.to_owned()));
        }

        eyre::ensure!(
            !target.starts_with("node:") && !NODE_BUILTINS.contains(&target),
            "{mode:?} resolving {specifier:?} from {}: external {target:?} requires explicit external policy",
            base.display()
        );
        let mut options = self.options.clone();
        options
            .export_conditions
            .retain(|c| c != "import" && c != "require");
        options.export_conditions.insert(
            0,
            match mode {
                ResolutionMode::Import => "import",
                ResolutionMode::Require => "require",
            }
            .to_owned(),
        );
        // Keep import/require cache lifetimes independent.
        let cache = match mode {
            ResolutionMode::Import => &self.import_cache,
            ResolutionMode::Require => &self.require_cache,
        };
        let result = cache
            .resolver(&self.root, options)
            .resolve(base, specifier)
            .map_err(|error| {
                eyre::eyre!(
                    "{:?} resolving {specifier:?} from {}: {error:#}",
                    mode,
                    base.display()
                )
            })?;
        eyre::ensure!(
            result.path.is_file(),
            "{mode:?} resolving {specifier:?} from {}: resolved target is not a file: {}",
            base.display(),
            result.path.display()
        );
        eyre::ensure!(
            self.options.target_env != TargetEnv::Browser
                || result.path.extension().is_none_or(|ext| ext != "node"),
            "{mode:?} resolving {specifier:?} from {}: native addon is unsupported for the browser target: {}",
            base.display(),
            result.path.display()
        );
        Ok(ResolvedDependency::File(result.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_builtin_aliases_are_exact_single_step_checked_files() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "stub.cjs" => "exports.value = 1;",
            "stub.mjs" => "export const value = 1;",
            "addon.node" => ""
        };
        for environment in [TargetEnv::Browser, TargetEnv::Node] {
            let mut options = NodeModulesResolverOptions::default_for_env(environment);
            options.alias.insert("fs".into(), "./stub.cjs".into());
            options.alias.insert(
                "node:fs".into(),
                dir.root().join("stub.mjs").to_str().unwrap().into(),
            );
            options
                .alias
                .insert("./stub.cjs".into(), "./missing.cjs".into());
            options.alias.insert("missing".into(), "./missing.cjs".into());
            options.alias.insert("cycle-a".into(), "cycle-b".into());
            options.alias.insert("cycle-b".into(), "cycle-a".into());
            options.alias.insert("self".into(), "self".into());
            options.alias.insert("native".into(), "node:fs".into());
            options.alias.insert("addon".into(), "./addon.node".into());
            let resolver = StructuralResolver::new(
                dir.root().into(),
                options,
                BTreeSet::from(["fs".into(), "node:fs".into()]),
            );
            for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                for (request, target) in [("fs", "stub.cjs"), ("node:fs", "stub.mjs")] {
                    assert_eq!(
                        resolver
                            .resolve_dependency(&dir.root().join("entry.mjs"), request, mode)
                            .unwrap(),
                        ResolvedDependency::File(dir.root().join(target)),
                    );
                }
                for request in [
                    "fs/promises",
                    "node:fs/promises",
                    "missing",
                    "cycle-a",
                    "self",
                    "native",
                ]
                .into_iter()
                .chain((environment == TargetEnv::Browser).then_some("addon"))
                {
                    let error = format!(
                        "{:#}",
                        resolver
                            .resolve_dependency(&dir.root().join("entry.mjs"), request, mode)
                            .unwrap_err()
                    );
                    assert!(
                        error.contains(request) && error.contains("entry.mjs"),
                        "{error}"
                    );
                }
            }
        }
    }

    #[test]
    fn package_string_roots_and_wildcard_resource_paths_resolve() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/closed/package.json" => r#"{"exports":"./index.mjs"}"#,
            "node_modules/closed/index.mjs" => "",
            "node_modules/closed/data/nested/file.json" => "{}",
            "node_modules/open/package.json" => r#"{"exports":{".":"./index.mjs","./*":"./*","./private/*":null}}"#,
            "node_modules/open/index.mjs" => "",
            "node_modules/open/data/nested/file.json" => "{}",
            "node_modules/open/private/file.json" => "{}"
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(crate::resolve::TargetEnv::Node),
            BTreeSet::new(),
        );
        for mode in [ResolutionMode::Import, ResolutionMode::Require] {
            for (request, target) in [
                ("closed", "closed/index.mjs"),
                ("closed/index.mjs", "closed/index.mjs"),
                (
                    "closed/data/nested/file.json",
                    "closed/data/nested/file.json",
                ),
                ("open", "open/index.mjs"),
                ("open/data/nested/file.json", "open/data/nested/file.json"),
            ] {
                assert_eq!(
                    resolver
                        .resolve_dependency(&dir.root().join("entry.mjs"), request, mode)
                        .unwrap(),
                    ResolvedDependency::File(dir.root().join("node_modules").join(target))
                );
            }
            for request in [
                "closed/missing.json",
                "closed/../open/index.mjs",
                "open/private/file.json",
                "open/missing.json",
            ] {
                assert!(
                    resolver
                        .resolve_dependency(&dir.root().join("entry.mjs"), request, mode)
                        .is_err(),
                    "{request}"
                );
            }
        }
    }

    #[test]
    fn browser_dependency_main_mapping_with_canonical_cwd_prefix() {
        const CHILD_ROOT: &str = "IMPORT_RESOLVER_BROWSER_CWD_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let cwd = PathBuf::from(root).canonicalize().unwrap();
            // Only this exact test runs in the child, so changing cwd is isolated.
            std::env::set_current_dir(&cwd).unwrap();
            assert_eq!(std::env::current_dir().unwrap(), cwd);
            let root = cwd.join("project");
            assert!(root
                .join("node_modules/dep/browser.mjs")
                .strip_prefix(&cwd)
                .is_ok());
            for (target, expected) in [
                (TargetEnv::Browser, "browser.mjs"),
                (TargetEnv::Node, "server.mjs"),
            ] {
                let mut options = NodeModulesResolverOptions::default_for_env(target);
                options.alias.insert("alias".into(), "dep".into());
                let resolver = StructuralResolver::new(root.clone(), options, BTreeSet::new());
                for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                    for request in ["dep", "alias", "dep"] {
                        assert_eq!(
                            resolver
                                .resolve_dependency(&root.join("entry.mjs"), request, mode)
                                .unwrap(),
                            ResolvedDependency::File(root.join("node_modules/dep").join(expected))
                        );
                    }
                }
            }
            return;
        }

        let dir = test_tmpdir::test_tmpdir! {
            "project/entry.mjs" => "",
            "project/node_modules/dep/package.json" => r#"{"main":"server.mjs","browser":{"./server.mjs":"./browser.mjs"}}"#,
            "project/node_modules/dep/server.mjs" => "",
            "project/node_modules/dep/browser.mjs" => ""
        };
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "structural::tests::browser_dependency_main_mapping_with_canonical_cwd_prefix",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ROOT, dir.root())
            .current_dir(dir.root())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated resolver test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn browser_dependency_main_mapping_and_importer_mapping_are_both_applied() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "package.json" => r#"{"browser":{"./local.mjs":"./local-browser.mjs"}}"#,
            "local.mjs" => "",
            "local-browser.mjs" => "",
            "node_modules/dep/package.json" => r#"{"main":"server.mjs","browser":{"./server.mjs":"./browser.mjs"}}"#,
            "node_modules/dep/server.mjs" => "",
            "node_modules/dep/browser.mjs" => ""
        };
        for (target, dependency, local) in [
            (TargetEnv::Browser, "browser.mjs", "local-browser.mjs"),
            (TargetEnv::Node, "server.mjs", "local.mjs"),
        ] {
            let mut options = NodeModulesResolverOptions::default_for_env(target);
            options.alias.insert("fs".into(), "./local.mjs".into());
            options.alias.insert(
                "node:fs".into(),
                dir.root().join("local.mjs").to_str().unwrap().into(),
            );
            let resolver = StructuralResolver::new(dir.root().into(), options, BTreeSet::new());
            for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                for (request, expected) in [
                    ("dep", dir.root().join("node_modules/dep").join(dependency)),
                    ("./local.mjs", dir.root().join(local)),
                    ("fs", dir.root().join(local)),
                    ("node:fs", dir.root().join(local)),
                ] {
                    assert_eq!(
                        resolver
                            .resolve_dependency(&dir.root().join("entry.mjs"), request, mode)
                            .unwrap(),
                        ResolvedDependency::File(expected)
                    );
                }
            }
        }
    }
}
