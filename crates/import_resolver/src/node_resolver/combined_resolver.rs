use std::path::Path;

use ftree_cache::context_data::FileContextCache;

use super::{
    caching::{
        CachingNodeModulesResolver, NodeModulesCache, NodeModulesResolverOptions, PackageJsonCache,
    },
    tsconfig::TsconfigConditions,
};
use crate::resolve::{PathResolver, Resolution};

type TsconfigCache = FileContextCache<TsconfigConditions, "tsconfig.json">;

#[derive(Debug, Default)]
pub struct CombinedResolverCaches {
    tsconfig_cache: TsconfigCache,
    node_modules_cache: NodeModulesCache,
    package_json_cache: PackageJsonCache,
}

impl CombinedResolverCaches {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolver<'a>(
        &'a self,
        root_dir: &'a Path,
        options: NodeModulesResolverOptions,
    ) -> CombinedResolver<'a> {
        CombinedResolver {
            root_dir,
            caches: self,
            options,
        }
    }

    pub fn mark_dirty_root(&mut self, root: &Path) {
        self.tsconfig_cache.mark_dirty_root(root);
        self.node_modules_cache.mark_dirty_root(root);
        self.package_json_cache.mark_dirty_root(root);
    }

    pub fn package_json_cache(&mut self) -> &mut PackageJsonCache {
        &mut self.package_json_cache
    }
}

/// Node/package resolution with additional tsconfig export conditions, never path rewrites.
pub struct CombinedResolver<'a> {
    root_dir: &'a Path,
    caches: &'a CombinedResolverCaches,
    options: NodeModulesResolverOptions,
}

impl PathResolver for CombinedResolver<'_> {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution> {
        let mut options = self.options.clone();
        if let Some((_, config)) = self.caches.tsconfig_cache.probe_path(self.root_dir, base)? {
            options
                .export_conditions
                .extend(config.value().0.iter().cloned());
        }
        CachingNodeModulesResolver::new(
            self.root_dir,
            &self.caches.package_json_cache,
            &self.caches.node_modules_cache,
            options,
        )
        .resolve(base, specifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::TargetEnv;
    use test_tmpdir::test_tmpdir;

    #[test]
    fn custom_conditions_are_scoped_and_cache_keys_include_the_active_set() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "nested/entry.mjs" => "",
            "tsconfig.json" => r#"{"compilerOptions":{"customConditions":["source"],"baseUrl":".","paths":{"pkg":["./wrong.mjs"]}}}"#,
            "nested/tsconfig.json" => r#"{"compilerOptions":{"customSourceConditions":["nested"]}}"#,
            "wrong.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":{"nested":"./nested.mjs","source":"./source.mjs","default":"./default.mjs"},"./blocked":{"nested":"./nested.mjs"}}}"#,
            "node_modules/pkg/nested.mjs" => "",
            "node_modules/pkg/source.mjs" => "",
            "node_modules/pkg/default.mjs" => ""
        };
        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            dir.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        for _ in 0..2 {
            for (base, expected) in [
                ("entry.mjs", "source.mjs"),
                ("nested/entry.mjs", "nested.mjs"),
            ] {
                assert_eq!(
                    resolver.resolve(&dir.root_join(base), "pkg").unwrap().path,
                    dir.root_join(&format!("node_modules/pkg/{expected}"))
                );
            }
        }
        assert!(resolver
            .resolve(&dir.root_join("entry.mjs"), "pkg/blocked")
            .is_err());
        assert_eq!(
            resolver
                .resolve(&dir.root_join("nested/entry.mjs"), "pkg/blocked")
                .unwrap()
                .path,
            dir.root_join("node_modules/pkg/nested.mjs")
        );
    }

    #[test]
    fn paths_and_base_url_never_create_aliases_or_local_fallbacks() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "tsconfig.json" => r#"{"compilerOptions":{"baseUrl":".","paths":{"alias":["./value.mjs"],"a*b*c":["./value.mjs"]}}}"#,
            "value.mjs" => ""
        };
        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            dir.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        for request in ["alias", "value"] {
            assert!(
                resolver
                    .resolve(&dir.root_join("entry.mjs"), request)
                    .is_err(),
                "{request}"
            );
        }
        assert_eq!(
            resolver
                .resolve(&dir.root_join("entry.mjs"), "./value.mjs")
                .unwrap()
                .path,
            dir.root_join("value.mjs")
        );
    }

    #[test]
    fn node_runtime_conditions_are_not_active_for_browser_targets() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":{"node-addons":"./node.mjs","module-sync":"./node.mjs","node":"./node.mjs","default":"./browser.mjs"}}}"#,
            "node_modules/pkg/node.mjs" => "",
            "node_modules/pkg/browser.mjs" => ""
        };
        for (environment, expected) in [
            (TargetEnv::Node, "node.mjs"),
            (TargetEnv::Browser, "browser.mjs"),
        ] {
            let caches = CombinedResolverCaches::new();
            let resolver = caches.resolver(
                dir.root(),
                NodeModulesResolverOptions::default_for_env(environment),
            );
            assert_eq!(
                resolver
                    .resolve(&dir.root_join("entry.mjs"), "pkg")
                    .unwrap()
                    .path,
                dir.root_join(&format!("node_modules/pkg/{expected}"))
            );
        }
    }
}
