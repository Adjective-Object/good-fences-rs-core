//! Explicit resolution policy for occurrence-aware graph consumers.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{
    node_resolver::{
        caching::NodeModulesResolverOptions, combined_resolver::CombinedResolverCaches,
    },
    resolve::{PathResolver, NODE_BUILTINS},
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
        if self.externals.contains(specifier) {
            return Ok(ResolvedDependency::External(specifier.to_owned()));
        }

        eyre::ensure!(
            !specifier.starts_with("node:") && !NODE_BUILTINS.contains(&specifier),
            "external {specifier:?} requires explicit external policy"
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
            "resolved target is not a file: {}",
            result.path.display()
        );
        Ok(ResolvedDependency::File(result.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_string_roots_and_wildcard_resource_paths_resolve() {
        let dir = test_tmpdir::test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/closed/package.json" => r#"{"exports":"./index.mjs"}"#,
            "node_modules/closed/index.mjs" => "",
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
                "closed/index.mjs",
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
}
