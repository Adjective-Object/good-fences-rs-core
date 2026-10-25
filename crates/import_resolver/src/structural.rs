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
use eyre::WrapErr;
pub use ftree_cache::inputs::{InputKind, InputProof, PathInput};
use ftree_cache::inputs::{InputObserver, InputSnapshot};

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

/// A resolution and the original filesystem observations behind it.
/// Consumers must freshly validate `inputs` before authorizing reuse or success.
#[derive(Debug, Clone)]
pub struct TrackedDependency {
    pub dependency: ResolvedDependency,
    pub inputs: InputProof,
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
    snapshot: InputSnapshot,
}

impl StructuralResolver {
    pub fn new(
        root: PathBuf,
        options: NodeModulesResolverOptions,
        externals: BTreeSet<String>,
    ) -> Self {
        let snapshot = InputSnapshot::default();
        let packages = std::sync::Arc::new(crate::node_resolver::caching::PackageJsonCache::new());
        Self {
            root,
            options,
            externals,
            import_cache: CombinedResolverCaches::with_shared(packages.clone(), snapshot.clone()),
            require_cache: CombinedResolverCaches::with_shared(packages, snapshot.clone()),
            revision: 0,
            snapshot,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Coarse invalidation is intentional: misses and configuration ancestry also change.
    pub fn invalidate(&mut self) {
        self.set_snapshot(InputSnapshot::default());
    }

    /// Replace the filesystem epoch and all context/target results together.
    pub fn set_snapshot(&mut self, snapshot: InputSnapshot) {
        let packages = std::sync::Arc::new(crate::node_resolver::caching::PackageJsonCache::new());
        self.import_cache = CombinedResolverCaches::with_shared(packages.clone(), snapshot.clone());
        self.require_cache = CombinedResolverCaches::with_shared(packages, snapshot.clone());
        self.snapshot = snapshot;
        self.revision += 1;
    }

    pub fn snapshot(&self) -> &InputSnapshot {
        &self.snapshot
    }

    pub fn set_policy(&mut self, options: NodeModulesResolverOptions, externals: BTreeSet<String>) {
        self.options = options;
        self.externals = externals;
        self.invalidate();
    }

    pub fn resolve_dependency_with_inputs(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<TrackedDependency> {
        let observer = InputObserver::with_snapshot(self.snapshot.clone());
        let dependency = self.resolve_inner(base, specifier, mode, Some(&observer))?;
        let inputs = observer.proof();
        inputs.check_errors().wrap_err_with(|| {
            format!(
                "{mode:?} resolving {specifier:?} from {}: checking input proof",
                base.display()
            )
        })?;
        Ok(TrackedDependency { dependency, inputs })
    }

    fn resolve_inner(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
        observer: Option<&InputObserver>,
    ) -> eyre::Result<ResolvedDependency> {
        // Check the same one-step exact replacement used by the file resolver.
        let replacement = self.options.alias.get(specifier);
        let target = replacement.map(String::as_str).unwrap_or(specifier);
        let browser = self.options.target_env == TargetEnv::Browser;
        let builtin = target.starts_with("node:") || NODE_BUILTINS.contains(&target);
        eyre::ensure!(
            !browser || !target.starts_with("node:"),
            "{mode:?} resolving {specifier:?} from {}: Node builtin {target:?} is unsupported for the browser target; no polyfills are provided",
            base.display()
        );
        // Browser bare builtins must prove a real package, never escape as externals.
        if replacement.is_none()
            && !target.starts_with('#')
            && self.externals.contains(specifier)
            && !(browser && builtin)
        {
            return Ok(ResolvedDependency::External(specifier.to_owned()));
        }

        eyre::ensure!(
            browser || !builtin,
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
        let resolver = cache.resolver(&self.root, options);
        let result = match observer {
            Some(observer) => resolver.resolve_with_inputs(base, specifier, observer.clone()),
            None => resolver.resolve(base, specifier),
        }
        .map_err(|error| eyre::eyre!(error.into_boxed_dyn_error()))
        .wrap_err_with(|| {
            if browser && builtin {
                format!(
                    "{mode:?} resolving {specifier:?} from {}: Node builtin {target:?} requires a resolvable installed package for the browser target; no polyfills are provided",
                    base.display()
                )
            } else {
                format!("{mode:?} resolving {specifier:?} from {}", base.display())
            }
        })?;
        eyre::ensure!(
            observer.map_or_else(
                || result.path.is_file(),
                |observer| observer.is_file(&result.path)
            ),
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

impl DependencyResolver for StructuralResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        self.resolve_inner(base, specifier, mode, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked(
        resolver: &StructuralResolver,
        base: &Path,
        request: &str,
        mode: ResolutionMode,
    ) -> TrackedDependency {
        let result = resolver
            .resolve_dependency_with_inputs(base, request, mode)
            .unwrap();
        result.inputs.validate().unwrap();
        result
    }

    #[test]
    fn package_imports_vfile_conditions_and_one_step_aliases() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r##"{"exports":{},"imports":{
                "#minpath":{"node":"node:path","default":"./lib/minpath.browser.js"},
                "#minproc":{"node":"node:process","default":"./lib/minproc.browser.js"},
                "#minurl":{"node":"node:url","default":"./lib/minurl.browser.js"},
                "#mode":{"custom":"./custom.js","import":"./import.js","require":"./require.js"},
                "#early":{"default":"./default.js","browser":"./missing.js"},
                "#nested":{"browser":{"inactive":"./missing.js"},"default":"./default.js"}
            }}"##,
            "entry.js" => "",
            "lib/minpath.browser.js" => "",
            "lib/minproc.browser.js" => "",
            "lib/minurl.browser.js" => "",
            "custom.js" => "",
            "import.js" => "",
            "require.js" => "",
            "default.js" => ""
        };
        let base = dir.root_join("entry.js");
        let caches = crate::node_resolver::combined_resolver::CombinedResolverCaches::new();
        for environment in [TargetEnv::Node, TargetEnv::Browser] {
            let resolver = caches.resolver(
                dir.root(),
                NodeModulesResolverOptions::default_for_env(environment),
            );
            for (request, builtin, file) in [
                ("#minpath", "node:path", "lib/minpath.browser.js"),
                ("#minproc", "node:process", "lib/minproc.browser.js"),
                ("#minurl", "node:url", "lib/minurl.browser.js"),
            ] {
                use crate::resolve::PathResolver;
                assert_eq!(
                    resolver.resolve(&base, request).unwrap().path,
                    if environment == TargetEnv::Node {
                        PathBuf::from(builtin)
                    } else {
                        dir.root_join(file)
                    }
                );
            }
        }
        for environment in [TargetEnv::Node, TargetEnv::Browser] {
            let mut options = NodeModulesResolverOptions::default_for_env(environment);
            options.alias.insert("aliased".into(), "#minpath".into());
            options.alias.insert("#minpath".into(), "#minurl".into());
            let mut resolver =
                StructuralResolver::new(dir.root().into(), options.clone(), BTreeSet::new());
            for name in ["minpath", "minproc", "minurl"] {
                let request = format!("#{name}");
                if environment == TargetEnv::Browser {
                    let expected = if name == "minpath" { "minurl" } else { name };
                    for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                        let cold = tracked(&resolver, &base, &request, mode);
                        let warm = tracked(&resolver, &base, &request, mode);
                        assert_eq!(cold.inputs, warm.inputs);
                        assert_eq!(
                            cold.dependency,
                            ResolvedDependency::File(
                                dir.root_join(&format!("lib/{expected}.browser.js"))
                            )
                        );
                    }
                } else {
                    let error = resolver
                        .resolve_dependency(&base, &request, ResolutionMode::Import)
                        .unwrap_err();
                    assert!(format!("{error:#}").contains(&request));
                }
            }
            if environment == TargetEnv::Browser {
                assert_eq!(
                    tracked(&resolver, &base, "aliased", ResolutionMode::Import).dependency,
                    ResolvedDependency::File(dir.root_join("lib/minpath.browser.js"))
                );
            }
            for (mode, file) in [
                (ResolutionMode::Import, "import.js"),
                (ResolutionMode::Require, "require.js"),
            ] {
                assert_eq!(
                    tracked(&resolver, &base, "#mode", mode).dependency,
                    ResolvedDependency::File(dir.root_join(file))
                );
            }
            for request in ["#early", "#nested"] {
                assert_eq!(
                    tracked(&resolver, &base, request, ResolutionMode::Import).dependency,
                    ResolvedDependency::File(dir.root_join("default.js"))
                );
            }
            options.export_conditions.push("custom".into());
            resolver.set_policy(options, BTreeSet::new());
            assert_eq!(
                tracked(&resolver, &base, "#mode", ResolutionMode::Require).dependency,
                ResolvedDependency::File(dir.root_join("custom.js"))
            );
        }
    }

    #[test]
    fn package_imports_patterns_fallbacks_and_errors() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r##"{"imports":{
                "#ok":"./ok.js",
                "#array":[false,42,"../outside.js","./%2e%2e/out.js",{"inactive":"./missing.js"},["./ok.js"]],
                "#nullarray":[null,"./ok.js"],
                "#null":{"import":null,"default":"./ok.js"},
                "#missing":["./missing.js","./ok.js"],
                "#inactive":{"inactive":"./ok.js"},
                "#empty":[],
                "#pattern/*":"./general/*.js",
                "#pattern/specific*":"./specific/*.js",
                "#pattern/*.js":"./suffix/*.js",
                "#pattern/exact":"./ok.js",
                "#repeat/*":"./*/*.js",
                "#prefix/":"./general/",
                "#escape/*":"./general/*",
                "#bad":"./node_modules/x.js",
                "#encoded":"./%6eode_modules/x.js",
                "#slash":"./x%2fy.js",
                "#backslash":"./x%5cy.js",
                "#absolute":"/outside.js",
                "#url":"https://example.org/file.js",
                "#malformed":"./%zz.js",
                "#numeric":{"0":"./ok.js","default":"./ok.js"},
                "#chain":"#ok",
                "#cycle":"#cycle2",
                "#cycle2":"#cycle"
            }}"##,
            "entry.js" => "",
            "ok.js" => "",
            "general/value.js" => "",
            "specific/value.js" => "",
            "suffix/value.js" => "",
            "value/value.js" => ""
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            ["#undefined".to_owned()].into_iter().collect(),
        );
        let base = dir.root_join("entry.js");
        for (request, expected) in [
            ("#ok", "ok.js"),
            ("#array", "ok.js"),
            ("#nullarray", "ok.js"),
            ("#chain", "ok.js"),
            ("#pattern/exact", "ok.js"),
            ("#pattern/value", "general/value.js"),
            ("#pattern/specificvalue", "specific/value.js"),
            ("#pattern/value.js", "suffix/value.js"),
            ("#repeat/value", "value/value.js"),
        ] {
            assert_eq!(
                tracked(&resolver, &base, request, ResolutionMode::Import).dependency,
                ResolvedDependency::File(dir.root_join(expected))
            );
        }
        for request in [
            "#",
            "#/",
            "#undefined",
            "#null",
            "#missing",
            "#inactive",
            "#empty",
            "#prefix/value",
            "#escape/../../ok.js",
            "#escape/%2e%2e/ok.js",
            "#bad",
            "#encoded",
            "#slash",
            "#backslash",
            "#absolute",
            "#url",
            "#malformed",
            "#numeric",
            "#cycle",
        ] {
            let error = resolver
                .resolve_dependency_with_inputs(&base, request, ResolutionMode::Import)
                .unwrap_err();
            let chain = format!("{error:#}");
            assert!(chain.contains(request), "{chain}");
            if request == "#cycle" {
                assert!(chain.contains("CYCLE"), "{chain}");
            }
        }
    }

    #[test]
    fn package_imports_scope_proofs_and_invalidation() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r##"{"imports":{"#value":"./old.js"}}"##,
            "old.js" => "",
            "new.js" => "",
            "nested/entry.js" => "",
            "nested/local.js" => "",
            "empty/package.json" => "{}",
            "empty/entry.js" => "",
            "node_modules/unscoped/entry.js" => "",
            "node_modules/@scope/unscoped/entry.js" => ""
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            BTreeSet::new(),
        );
        let base = dir.root_join("nested/entry.js");
        let cold = tracked(&resolver, &base, "#value", ResolutionMode::Import);
        assert_eq!(
            cold.inputs,
            tracked(&resolver, &base, "#value", ResolutionMode::Import).inputs
        );
        for entry in [
            "empty/entry.js",
            "node_modules/unscoped/entry.js",
            "node_modules/@scope/unscoped/entry.js",
        ] {
            assert!(resolver
                .resolve_dependency_with_inputs(
                    &dir.root_join(entry),
                    "#value",
                    ResolutionMode::Import
                )
                .is_err());
        }
        std::fs::write(
            dir.root_join("package.json"),
            r##"{"imports":{"#value":"./new.js"}}"##,
        )
        .unwrap();
        let stale = resolver
            .resolve_dependency_with_inputs(&base, "#value", ResolutionMode::Import)
            .unwrap();
        assert_eq!(cold.inputs, stale.inputs);
        assert!(stale.inputs.validate().is_err());
        resolver.invalidate();
        let edited = tracked(&resolver, &base, "#value", ResolutionMode::Import);
        assert_eq!(
            edited.dependency,
            ResolvedDependency::File(dir.root_join("new.js"))
        );
        std::fs::write(
            dir.root_join("nested/package.json"),
            r##"{"imports":{"#value":"./local.js"}}"##,
        )
        .unwrap();
        assert!(edited.inputs.validate().is_err());
        resolver.invalidate();
        let nearer = tracked(&resolver, &base, "#value", ResolutionMode::Import);
        assert_eq!(
            nearer.dependency,
            ResolvedDependency::File(dir.root_join("nested/local.js"))
        );
        std::fs::remove_file(dir.root_join("nested/package.json")).unwrap();
        assert!(nearer.inputs.validate().is_err());
        resolver.invalidate();
        let restored = tracked(&resolver, &base, "#value", ResolutionMode::Import);
        std::fs::remove_file(dir.root_join("new.js")).unwrap();
        assert!(restored.inputs.validate().is_err());
        resolver.invalidate();
        assert!(resolver
            .resolve_dependency_with_inputs(&base, "#value", ResolutionMode::Import)
            .is_err());
    }

    #[test]
    fn package_imports_apply_owning_browser_path_mapping() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r##"{"imports":{"#impl":"./server.js","#alias":"#impl","#disabled":"./disabled.js"},"browser":{"./server.js":"./browser.js","./disabled.js":false}}"##,
            "nested/entry.js" => "",
            "server.js" => "",
            "browser.js" => "",
            "disabled.js" => ""
        };
        for target in [TargetEnv::Browser, TargetEnv::Node] {
            let mut resolver = StructuralResolver::new(
                dir.root().into(),
                NodeModulesResolverOptions::default_for_env(target),
                BTreeSet::new(),
            );
            let base = dir.root_join("nested/entry.js");
            for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                let direct = tracked(&resolver, &base, "../server.js", mode);
                for request in ["#impl", "#alias"] {
                    let first = tracked(&resolver, &base, request, mode);
                    assert_eq!(first.dependency, direct.dependency);
                    assert_eq!(
                        first.dependency,
                        ResolvedDependency::File(dir.root_join(if target == TargetEnv::Browser {
                            "browser.js"
                        } else {
                            "server.js"
                        }))
                    );
                    assert_eq!(
                        first.inputs,
                        tracked(&resolver, &base, request, mode).inputs
                    );
                }
                assert_eq!(
                    tracked(&resolver, &base, "#disabled", mode).dependency,
                    ResolvedDependency::File(dir.root_join("disabled.js"))
                );
                // A directory base must retain its own manifest's browser scope.
                assert_eq!(
                    tracked(&resolver, dir.root(), "#impl", mode).dependency,
                    direct.dependency
                );
            }
            if target == TargetEnv::Browser {
                let first = tracked(&resolver, &base, "#impl", ResolutionMode::Import);
                std::fs::remove_file(dir.root_join("browser.js")).unwrap();
                assert!(first.inputs.validate().is_err());
                let stale = resolver
                    .resolve_dependency_with_inputs(&base, "#impl", ResolutionMode::Import)
                    .unwrap();
                assert_eq!(first.inputs, stale.inputs);
                assert!(stale.inputs.validate().is_err());
                resolver.invalidate();
                assert_eq!(
                    tracked(&resolver, &base, "#impl", ResolutionMode::Import).dependency,
                    ResolvedDependency::File(dir.root_join("server.js"))
                );
                std::fs::write(dir.root_join("browser.js"), "").unwrap();
            }
        }
    }

    #[test]
    fn package_imports_dependency_browser_mappings_keep_scopes_and_proofs() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r#"{"browser":{"./node_modules/owner/override.js":"./wrong.js"}}"#,
            "wrong.js" => "",
            "node_modules/owner/package.json" => r##"{"imports":{"#impl":"dep"},"browser":{"./node_modules/dep/browser.js":"./override.js"}}"##,
            "node_modules/owner/nested/entry.js" => "",
            "node_modules/owner/override.js" => "",
            "node_modules/owner/node_modules/dep/package.json" => r#"{"main":"server.js","browser":{"./server.js":"./browser.js"}}"#,
            "node_modules/owner/node_modules/dep/server.js" => "",
            "node_modules/owner/node_modules/dep/browser.js" => ""
        };
        let base = dir.root_join("node_modules/owner/nested/entry.js");
        for target in [TargetEnv::Browser, TargetEnv::Node] {
            let mut resolver = StructuralResolver::new(
                dir.root().into(),
                NodeModulesResolverOptions::default_for_env(target),
                BTreeSet::new(),
            );
            for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                let first = tracked(&resolver, &base, "#impl", mode);
                assert_eq!(
                    first.dependency,
                    ResolvedDependency::File(dir.root_join(if target == TargetEnv::Browser {
                        "node_modules/owner/override.js"
                    } else {
                        "node_modules/owner/node_modules/dep/server.js"
                    }))
                );
                assert_eq!(
                    first.dependency,
                    tracked(&resolver, &base, "dep", mode).dependency
                );
                let warm = tracked(&resolver, &base, "#impl", mode);
                assert_eq!(first.dependency, warm.dependency);
                assert_eq!(warm.inputs, tracked(&resolver, &base, "#impl", mode).inputs);
            }
            if target == TargetEnv::Browser {
                let first = tracked(&resolver, &base, "#impl", ResolutionMode::Import);
                std::fs::write(
                    dir.root_join("node_modules/owner/node_modules/dep/package.json"),
                    r#"{"main":"server.js","browser":{"./server.js":"./server.js"}}"#,
                )
                .unwrap();
                let stale = resolver
                    .resolve_dependency_with_inputs(&base, "#impl", ResolutionMode::Import)
                    .unwrap();
                assert_eq!(first.inputs, stale.inputs);
                assert!(stale.inputs.validate().is_err());
                resolver.invalidate();
                let changed = tracked(&resolver, &base, "#impl", ResolutionMode::Import);
                assert_eq!(
                    changed.dependency,
                    ResolvedDependency::File(
                        dir.root_join("node_modules/owner/node_modules/dep/server.js")
                    )
                );
                std::fs::write(
                    dir.root_join("node_modules/owner/package.json"),
                    r##"{"imports":{"#impl":"dep"},"browser":{"./node_modules/dep/server.js":"./override.js"}}"##,
                )
                .unwrap();
                assert!(changed.inputs.validate().is_err());
                resolver.invalidate();
                assert_eq!(
                    tracked(&resolver, &base, "#impl", ResolutionMode::Import).dependency,
                    ResolvedDependency::File(dir.root_join("node_modules/owner/override.js"))
                );
            }
        }
    }

    #[test]
    fn package_imports_dependency_redirects_keep_exports_and_browser_restrictions() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.js" => "",
            "package.json" => r##"{"imports":{"#value":"./wrong.js"}}"##,
            "wrong.js" => "",
            "node_modules/owner/package.json" => r##"{"imports":{
                "#value":"dep/value","#private":"dep/private","#fs":"fs","#node":"node:fs","#native":"./addon.node"
            }}"##,
            "node_modules/owner/entry.js" => "",
            "node_modules/owner/addon.node" => "",
            "node_modules/owner/node_modules/dep/package.json" => r#"{"exports":{"./value":{"import":"./import.js","require":"./require.js"}}}"#,
            "node_modules/owner/node_modules/dep/import.js" => "",
            "node_modules/owner/node_modules/dep/require.js" => "",
            "node_modules/owner/node_modules/dep/private.js" => "",
            "node_modules/owner/node_modules/fs/package.json" => r#"{"exports":"./browser.js"}"#,
            "node_modules/owner/node_modules/fs/browser.js" => ""
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            ["fs".to_owned(), "node:fs".to_owned()]
                .into_iter()
                .collect(),
        );
        let base = dir.root_join("node_modules/owner/entry.js");
        for (mode, expected) in [
            (ResolutionMode::Import, "import.js"),
            (ResolutionMode::Require, "require.js"),
        ] {
            let first = tracked(&resolver, &base, "#value", mode);
            assert_eq!(
                first.dependency,
                ResolvedDependency::File(
                    dir.root_join(&format!("node_modules/owner/node_modules/dep/{expected}"))
                )
            );
            assert_eq!(
                first.inputs,
                tracked(&resolver, &base, "#value", mode).inputs
            );
        }
        let original = tracked(&resolver, &base, "#value", ResolutionMode::Import);
        std::fs::write(
            dir.root_join("node_modules/owner/node_modules/dep/package.json"),
            r#"{"exports":{"./value":"./require.js"}}"#,
        )
        .unwrap();
        let stale = resolver
            .resolve_dependency_with_inputs(&base, "#value", ResolutionMode::Import)
            .unwrap();
        assert_eq!(original.inputs, stale.inputs);
        assert!(stale.inputs.validate().is_err());
        resolver.invalidate();
        assert_eq!(
            tracked(&resolver, &base, "#value", ResolutionMode::Import).dependency,
            ResolvedDependency::File(
                dir.root_join("node_modules/owner/node_modules/dep/require.js")
            )
        );
        for request in ["#private", "#node", "#native"] {
            assert!(resolver
                .resolve_dependency_with_inputs(&base, request, ResolutionMode::Import)
                .is_err());
        }
        let builtin = tracked(&resolver, &base, "#fs", ResolutionMode::Import);
        assert_eq!(
            builtin.dependency,
            ResolvedDependency::File(
                dir.root_join("node_modules/owner/node_modules/fs/browser.js")
            )
        );
        std::fs::remove_file(dir.root_join("node_modules/owner/node_modules/fs/browser.js"))
            .unwrap();
        assert!(builtin.inputs.validate().is_err());
        resolver.invalidate();
        assert!(resolver
            .resolve_dependency_with_inputs(&base, "#fs", ResolutionMode::Import)
            .is_err());
    }

    #[test]
    fn resolution_context_preserves_the_original_error_chain() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => ""
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let base = dir.root().join("entry.mjs");
        let error = resolver
            .resolve_dependency(&base, "./absent.mjs", ResolutionMode::Import)
            .unwrap_err();
        let chain = format!("{error:#}");
        assert!(chain.contains(&base.display().to_string()), "{chain}");
        assert!(chain.contains("./absent.mjs"), "{chain}");
        assert!(chain.contains("Import resolving"), "{chain}");
        assert!(error.chain().count() > 1, "{error:?}");
    }

    #[test]
    fn tracked_extension_priority_and_source_contents_are_distinct_inputs() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "value.ts" => "export const value = 1;"
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let result = tracked(
            &resolver,
            &dir.root().join("entry.mjs"),
            "./value",
            ResolutionMode::Import,
        );
        assert_eq!(
            result.dependency,
            ResolvedDependency::File(dir.root().join("value.ts"))
        );
        let candidates: Vec<_> = result
            .inputs
            .inputs()
            .iter()
            .filter(|input| input.path().file_stem().is_some_and(|stem| stem == "value"))
            .map(|input| (input.path().file_name().unwrap().to_owned(), input.kind()))
            .collect();
        assert_eq!(candidates[0], ("value.js".into(), InputKind::Missing));
        assert_eq!(candidates[1], ("value".into(), InputKind::Missing));
        assert_eq!(candidates[2], ("value.ts".into(), InputKind::File));
        std::fs::write(dir.root().join("value.ts"), "different source bytes").unwrap();
        result.inputs.validate().unwrap();
        std::fs::write(dir.root().join("value.js"), "").unwrap();
        assert!(result.inputs.validate().is_err());
        resolver.invalidate();
        assert_eq!(
            resolver
                .resolve_dependency(
                    &dir.root().join("entry.mjs"),
                    "./value",
                    ResolutionMode::Import
                )
                .unwrap(),
            ResolvedDependency::File(dir.root().join("value.js"))
        );
        assert!(resolver
            .resolve_dependency_with_inputs(
                &dir.root().join("entry.mjs"),
                "./absent",
                ResolutionMode::Import
            )
            .is_err());
    }

    #[test]
    fn tracked_cached_positive_retains_original_manifest_bytes() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":"./old.mjs"}"#,
            "node_modules/pkg/old.mjs" => "",
            "node_modules/pkg/new.mjs" => ""
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let base = dir.root().join("entry.mjs");
        // A legacy call also seals the provenance of the shared cache's values.
        let original = resolver
            .resolve_dependency(&base, "pkg", ResolutionMode::Import)
            .unwrap();
        let first = tracked(&resolver, &base, "pkg", ResolutionMode::Import);
        std::fs::write(
            dir.root().join("node_modules/pkg/package.json"),
            r#"{"exports":"./new.mjs"}"#,
        )
        .unwrap();
        let cached = resolver
            .resolve_dependency_with_inputs(&base, "pkg", ResolutionMode::Import)
            .unwrap();
        assert_eq!(cached.dependency, original);
        assert_eq!(cached.inputs, first.inputs);
        assert!(cached.inputs.validate().is_err());
        resolver.invalidate();
        assert_eq!(
            tracked(&resolver, &base, "pkg", ResolutionMode::Import).dependency,
            ResolvedDependency::File(dir.root().join("node_modules/pkg/new.mjs"))
        );
    }

    #[test]
    fn warm_package_resolution_does_not_reopen_a_cached_target_for_another_importer() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "nested/entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":"./index.mjs"}"#,
            "node_modules/pkg/index.mjs" => ""
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let first = tracked(
            &resolver,
            &dir.root().join("entry.mjs"),
            "pkg",
            ResolutionMode::Import,
        );
        std::fs::remove_file(dir.root().join("node_modules/pkg/index.mjs")).unwrap();
        let cached = resolver
            .resolve_dependency_with_inputs(
                &dir.root().join("nested/entry.mjs"),
                "pkg",
                ResolutionMode::Import,
            )
            .unwrap();
        assert_eq!(cached.dependency, first.dependency);
        assert!(cached.inputs.validate().is_err());
        resolver.invalidate();
        assert!(resolver
            .resolve_dependency_with_inputs(
                &dir.root().join("nested/entry.mjs"),
                "pkg",
                ResolutionMode::Import
            )
            .is_err());
    }

    #[test]
    fn tracked_ancestor_misses_and_cached_negative_detect_nearer_packages() {
        for present_node_modules in [false, true] {
            let dir = test_tmpdir::test_tmpdir! {
                "nested/entry.mjs" => "",
                "node_modules/pkg/index.js" => ""
            };
            if present_node_modules {
                std::fs::create_dir(dir.root().join("nested/node_modules")).unwrap();
            }
            let resolver = StructuralResolver::new(
                dir.root().into(),
                NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
                BTreeSet::new(),
            );
            let base = dir.root().join("nested/entry.mjs");
            let result = tracked(&resolver, &base, "pkg", ResolutionMode::Import);
            // Cache hits must retain both ancestor context misses and target misses.
            let cached = tracked(&resolver, &base, "pkg", ResolutionMode::Import);
            assert_eq!(result.inputs, cached.inputs);
            std::fs::create_dir_all(dir.root().join("nested/node_modules/pkg")).unwrap();
            std::fs::write(dir.root().join("nested/node_modules/pkg/index.js"), "").unwrap();
            assert!(cached.inputs.validate().is_err());
            let after = resolver
                .resolve_dependency_with_inputs(&base, "pkg", ResolutionMode::Import)
                .unwrap();
            assert!(after.inputs.validate().is_err());
        }
    }

    #[test]
    fn exports_arrays_resolve_modes_patterns_and_null_fallbacks_with_checked_cache_hits() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":[{"browser":"./browser.mjs","import":"./import.mjs","require":"./require.cjs"},"./fallback.mjs"],"./helpers/*":[{"browser":"./browser/*.mjs"},false,"./helpers/*.mjs"],"./nullable":[null,"./fallback.mjs"],"./empty":[],"./missing":["./absent.mjs","./fallback.mjs"]}}"#,
            "node_modules/pkg/browser.mjs" => "",
            "node_modules/pkg/import.mjs" => "",
            "node_modules/pkg/require.cjs" => "",
            "node_modules/pkg/fallback.mjs" => "",
            "node_modules/pkg/helpers/item.mjs" => "",
            "node_modules/pkg/browser/item.mjs" => "",
            "node_modules/pkg/empty.js" => ""
        };
        let base = dir.root().join("entry.mjs");
        for environment in [TargetEnv::Node, TargetEnv::Browser] {
            let resolver = StructuralResolver::new(
                dir.root().into(),
                NodeModulesResolverOptions::default_for_env(environment),
                BTreeSet::new(),
            );
            for _ in 0..2 {
                for (request, mode, expected) in [
                    ("pkg", ResolutionMode::Import, "import.mjs"),
                    ("pkg", ResolutionMode::Require, "require.cjs"),
                    (
                        "pkg/helpers/item",
                        ResolutionMode::Import,
                        "helpers/item.mjs",
                    ),
                    ("pkg/nullable", ResolutionMode::Import, "fallback.mjs"),
                ] {
                    let expected = if environment == TargetEnv::Browser {
                        match expected {
                            "import.mjs" | "require.cjs" => "browser.mjs",
                            "helpers/item.mjs" => "browser/item.mjs",
                            _ => expected,
                        }
                    } else {
                        expected
                    };
                    assert_eq!(
                        tracked(&resolver, &base, request, mode).dependency,
                        ResolvedDependency::File(
                            dir.root().join("node_modules/pkg").join(expected)
                        )
                    );
                }
                for request in ["pkg/empty", "pkg/missing", "pkg/private"] {
                    let error = resolver
                        .resolve_dependency(&base, request, ResolutionMode::Import)
                        .unwrap_err();
                    assert!(format!("{error:#}").contains(request), "{error:#}");
                }
            }
        }
    }

    #[test]
    fn nested_export_conditions_resolve_and_cache_import_require_and_custom_modes() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":{"iconFont":{"import":{"types":"./fonts.d.ts","default":"./fonts.mjs"}},"default":{"import":{"types":"./index.d.ts","default":"./index.mjs"},"require":{"types":"./index.d.cts","default":"./index.cjs"}}},"./svg/*":{"import":{"types":"./atoms/*.d.ts","default":"./atoms/*.mjs"},"require":{"types":"./atoms/*.d.cts","default":"./atoms/*.cjs"}}}}"#,
            "node_modules/pkg/index.mjs" => "",
            "node_modules/pkg/index.cjs" => "",
            "node_modules/pkg/fonts.mjs" => "",
            "node_modules/pkg/atoms/icon.mjs" => "",
            "node_modules/pkg/atoms/icon.cjs" => ""
        };
        for environment in [TargetEnv::Node, TargetEnv::Browser] {
            let mut resolver = StructuralResolver::new(
                dir.root().into(),
                NodeModulesResolverOptions::default_for_env(environment),
                BTreeSet::new(),
            );
            for _ in 0..2 {
                for (request, mode, file) in [
                    ("pkg", ResolutionMode::Import, "index.mjs"),
                    ("pkg", ResolutionMode::Require, "index.cjs"),
                    ("pkg/svg/icon", ResolutionMode::Import, "atoms/icon.mjs"),
                    ("pkg/svg/icon", ResolutionMode::Require, "atoms/icon.cjs"),
                ] {
                    let result = tracked(&resolver, &dir.root().join("entry.mjs"), request, mode);
                    assert_eq!(
                        result.dependency,
                        ResolvedDependency::File(dir.root().join("node_modules/pkg").join(file))
                    );
                }
            }
            let mut options = NodeModulesResolverOptions::default_for_env(environment);
            options.export_conditions.push("iconFont".into());
            resolver.set_policy(options, BTreeSet::new());
            assert_eq!(
                tracked(
                    &resolver,
                    &dir.root().join("entry.mjs"),
                    "pkg",
                    ResolutionMode::Import
                )
                .dependency,
                ResolvedDependency::File(dir.root().join("node_modules/pkg/fonts.mjs"))
            );
        }
    }

    #[test]
    fn tracked_manifest_cache_and_request_modes_keep_original_provenance() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":{"import":"./import.mjs","require":"./require.cjs"},"./other":"./import.mjs"}}"#,
            "node_modules/pkg/import.mjs" => "",
            "node_modules/pkg/require.cjs" => ""
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let base = dir.root().join("entry.mjs");
        for (mode, path) in [
            (ResolutionMode::Import, "import.mjs"),
            (ResolutionMode::Require, "require.cjs"),
        ] {
            let result = tracked(&resolver, &base, "pkg", mode);
            assert_eq!(
                result.dependency,
                ResolvedDependency::File(dir.root().join("node_modules/pkg").join(path))
            );
        }
        std::fs::write(
            dir.root().join("node_modules/pkg/package.json"),
            r#"{"exports":{".":"./require.cjs","./other":"./require.cjs"}}"#,
        )
        .unwrap();
        // A different uncached request still uses the old cached manifest.
        let result = resolver
            .resolve_dependency_with_inputs(&base, "pkg/other", ResolutionMode::Import)
            .unwrap();
        assert_eq!(
            result.dependency,
            ResolvedDependency::File(dir.root().join("node_modules/pkg/import.mjs"))
        );
        assert!(result.inputs.validate().is_err());
    }

    #[test]
    fn tracked_browser_rewrite_cache_retains_candidate_and_manifest_inputs() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "package.json" => r#"{"browser":{"./optional.js":"./browser.mjs"}}"#,
            "value.mjs" => "",
            "other.mjs" => "",
            "browser.mjs" => ""
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            BTreeSet::new(),
        );
        let base = dir.root().join("entry.mjs");
        let first = tracked(&resolver, &base, "./value.mjs", ResolutionMode::Import);
        std::fs::write(dir.root().join("optional.js"), "").unwrap();
        assert!(first.inputs.validate().is_err());
        let derived_hit = resolver
            .resolve_dependency_with_inputs(&base, "./other.mjs", ResolutionMode::Import)
            .unwrap();
        assert!(derived_hit.inputs.validate().is_err());
        std::fs::remove_file(dir.root().join("optional.js")).unwrap();
        derived_hit.inputs.validate().unwrap();
        std::fs::write(
            dir.root().join("package.json"),
            r#"{"browser":{"./value.mjs":"./browser.mjs"}}"#,
        )
        .unwrap();
        assert!(derived_hit.inputs.validate().is_err());
    }

    #[test]
    fn tracked_browser_scope_misses_detect_a_nearer_manifest() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => "{}",
            "nested/entry.mjs" => "",
            "nested/value.mjs" => ""
        };
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            BTreeSet::new(),
        );
        let result = tracked(
            &resolver,
            &dir.root().join("nested/entry.mjs"),
            "./value.mjs",
            ResolutionMode::Import,
        );
        std::fs::write(dir.root().join("nested/package.json"), "{}").unwrap();
        assert!(result.inputs.validate().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tracked_api_does_not_hide_probe_io_errors_behind_a_successful_fallback() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "value.ts" => ""
        };
        std::os::unix::fs::symlink("value.js", dir.root().join("value.js")).unwrap();
        let resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            BTreeSet::new(),
        );
        let base = dir.root().join("entry.mjs");
        assert!(resolver
            .resolve_dependency(&base, "./value", ResolutionMode::Import)
            .is_ok());
        let error = resolver
            .resolve_dependency_with_inputs(&base, "./value", ResolutionMode::Import)
            .unwrap_err();
        assert!(format!("{error:#}").contains("unproven resolution inputs"));
        std::fs::remove_file(dir.root().join("value.js")).unwrap();
    }

    #[cfg(any(unix, windows))]
    fn link_directory(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn tracked_intermediate_link_retarget_is_visible_under_both_path_policies() {
        for preserve in [false, true] {
            let dir = test_tmpdir::test_tmpdir! {
                "entry.mjs" => "",
                "first/deep/value.mjs" => "",
                "second/deep/value.mjs" => ""
            };
            let link = dir.root().join("linked");
            link_directory(&dir.root().join("first"), &link);
            let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
            options.preserve_symlinks = preserve;
            let resolver = StructuralResolver::new(dir.root().into(), options, BTreeSet::new());
            let result = tracked(
                &resolver,
                &dir.root().join("entry.mjs"),
                "./linked/deep/value.mjs",
                ResolutionMode::Import,
            );
            #[cfg(windows)]
            std::fs::remove_dir(&link).unwrap();
            #[cfg(unix)]
            std::fs::remove_file(&link).unwrap();
            link_directory(&dir.root().join("second"), &link);
            assert!(result.inputs.validate().is_err());
            #[cfg(windows)]
            std::fs::remove_dir(&link).unwrap();
            #[cfg(unix)]
            std::fs::remove_file(&link).unwrap();
        }
    }

    #[test]
    fn browser_bare_builtin_packages_use_mode_exports_and_tracked_invalidation() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/buffer/package.json" => r#"{"exports":{".":{"browser":{"import":"./import.mjs","require":"./require.cjs"}},"./private":null}}"#,
            "node_modules/buffer/import.mjs" => "",
            "node_modules/buffer/require.cjs" => "",
            "node_modules/buffer/next.mjs" => "",
            "node_modules/buffer/private.js" => ""
        };
        let mut resolver = StructuralResolver::new(
            dir.root().into(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Browser),
            BTreeSet::from(["buffer".into()]),
        );
        let base = dir.root().join("entry.mjs");
        let mut proofs = Vec::new();
        for (mode, file) in [
            (ResolutionMode::Import, "import.mjs"),
            (ResolutionMode::Require, "require.cjs"),
        ] {
            let first = tracked(&resolver, &base, "buffer", mode);
            assert_eq!(
                first.dependency,
                ResolvedDependency::File(dir.root().join("node_modules/buffer").join(file))
            );
            assert_eq!(
                tracked(&resolver, &base, "buffer", mode).inputs,
                first.inputs
            );
            assert!(resolver
                .resolve_dependency(&base, "buffer/private", mode)
                .is_err());
            proofs.push(first);
        }
        std::fs::write(
            dir.root().join("node_modules/buffer/package.json"),
            r#"{"exports":"./next.mjs"}"#,
        )
        .unwrap();
        for proof in &proofs {
            assert!(proof.inputs.validate().is_err());
        }
        resolver.invalidate();
        for mode in [ResolutionMode::Import, ResolutionMode::Require] {
            assert_eq!(
                tracked(&resolver, &base, "buffer", mode).dependency,
                ResolvedDependency::File(dir.root().join("node_modules/buffer/next.mjs"))
            );
        }
        std::fs::remove_file(dir.root().join("node_modules/buffer/next.mjs")).unwrap();
        resolver.invalidate();
        for mode in [ResolutionMode::Import, ResolutionMode::Require] {
            let error = resolver
                .resolve_dependency(&base, "buffer", mode)
                .unwrap_err();
            assert!(error.chain().count() > 1, "{error:?}");
            let chain = format!("{error:#}");
            assert!(
                chain.contains("Node builtin") && chain.contains("entry.mjs"),
                "{chain}"
            );
        }
        std::fs::write(dir.root().join("node_modules/buffer/next.mjs"), "").unwrap();
        resolver.invalidate();
        for mode in [ResolutionMode::Import, ResolutionMode::Require] {
            tracked(&resolver, &base, "buffer", mode);
        }
    }

    #[test]
    fn browser_bare_builtin_packages_ignore_competing_cwd_files() {
        const CHILD_ROOT: &str = "IMPORT_RESOLVER_BUILTIN_CWD_ROOT";
        if std::env::var_os(CHILD_ROOT).is_some() {
            let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
            for project in ["installed", "private", "absent"] {
                let root = cwd.join(project);
                let base = root.join("entry.mjs");
                let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
                options.alias.insert("shim".into(), "buffer".into());
                options
                    .alias
                    .insert("node:buffer".into(), "./stub.cjs".into());
                let resolver = StructuralResolver::new(
                    root.clone(),
                    options,
                    BTreeSet::from(["buffer".into(), "shim".into(), "node:buffer".into()]),
                );
                for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                    for request in ["buffer", "shim", "buffer"] {
                        if project == "installed" {
                            let expected = ResolvedDependency::File(root.join(match mode {
                                ResolutionMode::Import => "node_modules/buffer/import.mjs",
                                ResolutionMode::Require => "node_modules/buffer/require.cjs",
                            }));
                            assert_eq!(
                                resolver.resolve_dependency(&base, request, mode).unwrap(),
                                expected
                            );
                            let result = tracked(&resolver, &base, request, mode);
                            assert_eq!(result.dependency, expected);
                            assert!(result.inputs.inputs().iter().any(|input| {
                                input.path() == root.join("node_modules/buffer/package.json")
                            }));
                            assert!(!result
                                .inputs
                                .inputs()
                                .iter()
                                .any(|input| input.path() == cwd.join("buffer.js")));
                        } else {
                            for error in [
                                resolver
                                    .resolve_dependency(&base, request, mode)
                                    .unwrap_err(),
                                resolver
                                    .resolve_dependency_with_inputs(&base, request, mode)
                                    .unwrap_err(),
                            ] {
                                let chain = format!("{error:#}");
                                assert!(
                                    chain.contains("requires a resolvable installed package")
                                        && chain.contains(request)
                                        && chain.contains(&base.display().to_string()),
                                    "{chain}"
                                );
                                if project == "private" {
                                    assert!(chain.contains("marked as private"), "{chain}");
                                }
                            }
                        }
                    }
                    assert_eq!(
                        tracked(&resolver, &base, "node:buffer", mode).dependency,
                        ResolvedDependency::File(root.join("stub.cjs"))
                    );
                }
            }
            return;
        }

        let dir = test_tmpdir::test_tmpdir! {
            "buffer.js" => "throw new Error('cwd must not shadow installed packages');",
            "installed/entry.mjs" => "",
            "installed/stub.cjs" => "",
            "installed/node_modules/buffer/package.json" => r#"{"exports":{".":{"import":"./import.mjs","require":"./require.cjs"}}}"#,
            "installed/node_modules/buffer/import.mjs" => "",
            "installed/node_modules/buffer/require.cjs" => "",
            "private/entry.mjs" => "",
            "private/stub.cjs" => "",
            "private/node_modules/buffer/package.json" => r#"{"exports":{".":null}}"#,
            "private/node_modules/buffer/index.js" => "",
            "absent/entry.mjs" => "",
            "absent/stub.cjs" => ""
        };
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "structural::tests::browser_bare_builtin_packages_ignore_competing_cwd_files",
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
    fn browser_bare_builtin_packages_do_not_relax_node_scheme_native_or_node_policy() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "stub.cjs" => "",
            "node_modules/buffer/package.json" => r#"{"main":"index.js"}"#,
            "node_modules/buffer/index.js" => "exports.Buffer = {};",
            "node_modules/path/package.json" => r#"{"main":"addon.node"}"#,
            "node_modules/path/addon.node" => ""
        };
        let base = dir.root().join("entry.mjs");
        for environment in [TargetEnv::Browser, TargetEnv::Node] {
            for external in [false, true] {
                let mut options = NodeModulesResolverOptions::default_for_env(environment);
                options
                    .alias
                    .insert("node:aliased".into(), "./stub.cjs".into());
                options.alias.insert("shim".into(), "buffer".into());
                let resolver = StructuralResolver::new(
                    dir.root().into(),
                    options,
                    if external {
                        ["buffer", "node:buffer", "fs", "path"]
                            .map(String::from)
                            .into()
                    } else {
                        BTreeSet::new()
                    },
                );
                for mode in [ResolutionMode::Import, ResolutionMode::Require] {
                    assert_eq!(
                        tracked(&resolver, &base, "node:aliased", mode).dependency,
                        ResolvedDependency::File(dir.root().join("stub.cjs"))
                    );
                    if environment == TargetEnv::Node {
                        for request in ["buffer", "node:buffer", "fs", "path"] {
                            let result = resolver.resolve_dependency(&base, request, mode);
                            if external {
                                assert_eq!(
                                    result.unwrap(),
                                    ResolvedDependency::External(request.into())
                                );
                            } else {
                                assert!(result.is_err());
                            }
                        }
                        assert!(resolver.resolve_dependency(&base, "shim", mode).is_err());
                        continue;
                    }
                    for request in ["buffer", "shim"] {
                        assert_eq!(
                            tracked(&resolver, &base, request, mode).dependency,
                            ResolvedDependency::File(
                                dir.root().join("node_modules/buffer/index.js")
                            )
                        );
                    }
                    for (request, message) in [
                        ("node:buffer", "Node builtin"),
                        ("fs", "Node builtin"),
                        ("path", "native addon"),
                    ] {
                        let error = resolver
                            .resolve_dependency(&base, request, mode)
                            .unwrap_err();
                        let chain = format!("{error:#}");
                        assert!(
                            chain.contains(message)
                                && chain.contains(request)
                                && chain.contains("entry.mjs"),
                            "{chain}"
                        );
                        if request == "fs" {
                            assert!(error.chain().count() > 1, "{error:?}");
                        }
                    }
                }
            }
        }
    }

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
            options
                .alias
                .insert("missing".into(), "./missing.cjs".into());
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
