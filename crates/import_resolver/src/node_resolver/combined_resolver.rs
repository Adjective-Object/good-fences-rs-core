use ftree_cache::inputs::InputSnapshot;
use std::{path::Path, sync::Arc};

use super::caching::{
    CachingNodeModulesResolver, NodeModulesCache, NodeModulesResolverOptions, PackageJsonCache,
};
use crate::resolve::{PathResolver, Resolution};

#[derive(Debug, Default)]
pub struct CombinedResolverCaches {
    node_modules_cache: NodeModulesCache,
    package_json_cache: Arc<PackageJsonCache>,
    snapshot: InputSnapshot,
}

impl CombinedResolverCaches {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_shared(
        package_json_cache: Arc<PackageJsonCache>,
        snapshot: InputSnapshot,
    ) -> Self {
        Self {
            package_json_cache,
            snapshot,
            node_modules_cache: NodeModulesCache::new(),
        }
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
        self.node_modules_cache.mark_dirty_root(root);
        self.package_json_cache.mark_dirty_root(root);
        self.snapshot.mark_dirty_root(root);
    }

    pub fn package_json_cache(&mut self) -> &PackageJsonCache {
        &self.package_json_cache
    }
}

/// Node/package resolution with explicitly configured export conditions.
pub struct CombinedResolver<'a> {
    root_dir: &'a Path,
    caches: &'a CombinedResolverCaches,
    options: NodeModulesResolverOptions,
}

impl PathResolver for CombinedResolver<'_> {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution> {
        CachingNodeModulesResolver::new(
            self.root_dir,
            &self.caches.package_json_cache,
            &self.caches.node_modules_cache,
            self.options.clone(),
        )
        .with_observer(ftree_cache::inputs::InputObserver::with_snapshot(
            self.caches.snapshot.clone(),
        ))
        .resolve(base, specifier)
    }
}

impl CombinedResolver<'_> {
    pub(crate) fn resolve_with_inputs(
        &self,
        base: &Path,
        specifier: &str,
        observer: ftree_cache::inputs::InputObserver,
    ) -> anyhow::Result<Resolution> {
        CachingNodeModulesResolver::new(
            self.root_dir,
            &self.caches.package_json_cache,
            &self.caches.node_modules_cache,
            self.options.clone(),
        )
        .with_observer(observer)
        .resolve(base, specifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::TargetEnv;
    use test_tmpdir::test_tmpdir;

    #[test]
    fn explicit_aliases_precede_node_builtins_without_recursive_rewriting() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "stub.cjs" => ""
        };
        for environment in [TargetEnv::Node, TargetEnv::Browser] {
            let caches = CombinedResolverCaches::new();
            let mut options = NodeModulesResolverOptions::default_for_env(environment);
            options.alias.insert("fs".into(), "./stub.cjs".into());
            options.alias.insert("node:fs".into(), "./stub.cjs".into());
            options
                .alias
                .insert("./stub.cjs".into(), "./missing.cjs".into());
            let resolver = caches.resolver(dir.root(), options);
            for request in ["fs", "node:fs"] {
                assert_eq!(
                    resolver
                        .resolve(&dir.root_join("entry.mjs"), request)
                        .unwrap()
                        .path,
                    dir.root_join("stub.cjs"),
                );
            }
        }
        let caches = CombinedResolverCaches::new();
        let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
        options.alias.insert("native".into(), "fs".into());
        let resolver = caches.resolver(dir.root(), options);
        for request in ["native", "fs", "node:fs"] {
            assert_eq!(
                resolver
                    .resolve(&dir.root_join("entry.mjs"), request)
                    .unwrap()
                    .path,
                std::path::PathBuf::from("node:fs"),
            );
        }
    }

    #[test]
    fn explicit_conditions_key_shared_caches_and_tsconfigs_have_no_effect() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "nested/entry.mjs" => "",
            "tsconfig.json" => r#"{
                // Compiler-only configuration, including unsupported inheritance.
                "extends": "./missing.json",
                "references": [{"path":"./missing-project"},],
                "compilerOptions": {"customConditions":["source"],"baseUrl":".","paths":{"pkg":["./wrong.mjs"]},},
            }"#,
            "nested/tsconfig.json" => r#"{"compilerOptions":{"customSourceConditions":["nested"]}}"#,
            "wrong.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":{".":{"nested":"./nested.mjs","source":"./source.mjs","default":"./default.mjs"},"./blocked":{"nested":"./nested.mjs"}}}"#,
            "node_modules/pkg/nested.mjs" => "",
            "node_modules/pkg/source.mjs" => "",
            "node_modules/pkg/default.mjs" => ""
        };
        let caches = CombinedResolverCaches::new();
        for _ in 0..2 {
            for (condition, expected) in [
                (None, "default.mjs"),
                (Some("source"), "source.mjs"),
                (Some("nested"), "nested.mjs"),
            ] {
                let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
                options
                    .export_conditions
                    .extend(condition.map(str::to_owned));
                let resolver = caches.resolver(dir.root(), options);
                for base in ["entry.mjs", "nested/entry.mjs"] {
                    assert_eq!(
                        resolver.resolve(&dir.root_join(base), "pkg").unwrap().path,
                        dir.root_join(&format!("node_modules/pkg/{expected}"))
                    );
                    let blocked = resolver.resolve(&dir.root_join(base), "pkg/blocked");
                    if condition == Some("nested") {
                        assert_eq!(
                            blocked.unwrap().path,
                            dir.root_join("node_modules/pkg/nested.mjs")
                        );
                    } else {
                        assert!(blocked.is_err());
                    }
                }
            }
        }
    }

    #[test]
    fn concurrent_package_targets_keep_extension_policies_separate() {
        let dir = test_tmpdir! {
            "entry.mjs" => "",
            "node_modules/pkg/package.json" => r#"{"exports":"./value"}"#,
            "node_modules/pkg/value.ts" => "",
            "node_modules/pkg/value.tsx" => ""
        };
        let caches = CombinedResolverCaches::new();
        std::thread::scope(|scope| {
            for extension in ["ts", "tsx", "ts", "tsx"] {
                let caches = &caches;
                let dir = &dir;
                scope.spawn(move || {
                    let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
                    options.extensions = vec![extension.into()];
                    let resolver = caches.resolver(dir.root(), options);
                    for _ in 0..8 {
                        assert_eq!(
                            resolver
                                .resolve(&dir.root_join("entry.mjs"), "pkg")
                                .unwrap()
                                .path,
                            dir.root_join(&format!("node_modules/pkg/value.{extension}"))
                        );
                    }
                });
            }
        });
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
