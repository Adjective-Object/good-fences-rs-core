use anyhow::{anyhow, Error};
use std::path::Path;
use swc_common::FileName;
use swc_core::ecma::loader::resolve::Resolution;
use swc_ecma_loader::resolve::Resolve;

use super::{
    node_resolver::{
        CachingNodeModulesResolver, NodeModulesCache, NodeModulesResolverOptions, PackageJsonCache,
    },
    tsconfig::ProcessedTsconfig,
    tsconfig_resolver::{TsconfigCache, TsconfigPathsResolver},
};

// Caches used by the Combined resolver's sub-resolvers
#[derive(Debug)]
pub struct CombinedResolverCaches {
    tsconfig_cache: TsconfigCache,
    node_modules_cache: NodeModulesCache,
    package_json_cache: PackageJsonCache,
}

impl CombinedResolverCaches {
    pub fn new() -> Self {
        Self {
            node_modules_cache: NodeModulesCache::new(),
            package_json_cache: PackageJsonCache::new(),
            tsconfig_cache: TsconfigCache::new(),
        }
    }

    pub fn resolver<'a>(
        &'a self,
        root_dir: &'a Path,
        options: NodeModulesResolverOptions,
    ) -> CombinedResolver<'a> {
        CombinedResolver::<'a> {
            root_dir,
            tsconfig_cache: &self.tsconfig_cache,
            node_modules_resolver: CachingNodeModulesResolver::new(
                root_dir,
                &self.package_json_cache,
                &self.node_modules_cache,
                options,
            ),
        }
    }

    /// Mark the files files in the given path as dirty
    pub fn mark_dirty_root(&mut self, root: &Path) {
        self.tsconfig_cache.mark_dirty_root(root);
        self.node_modules_cache.mark_dirty_root(root);
        self.package_json_cache.mark_dirty_root(root);
    }

    // pre-populate a package json cache with a package.json file
    pub fn package_json_cache(&mut self) -> &mut PackageJsonCache {
        &mut self.package_json_cache
    }
}

impl Default for CombinedResolverCaches {
    fn default() -> Self {
        Self::new()
    }
}

// Resolver that combines the tsconfig and node_modules resolvers,
// preferring resolution from the tsconfig resolver when possible
pub struct CombinedResolver<'a> {
    root_dir: &'a Path,
    tsconfig_cache: &'a TsconfigCache,
    node_modules_resolver: CachingNodeModulesResolver<'a>,
}

impl<'a> Resolve for CombinedResolver<'a> {
    fn resolve(&self, base: &FileName, module_specifier: &str) -> Result<Resolution, Error> {
        let base_path = match base {
            FileName::Real(path) => path,
            _ => return Err(anyhow!("Base must be a real file path")),
        };

        match self.tsconfig_cache.probe_path(self.root_dir, base_path)? {
            Some((path, maybe_tsconfig)) => match maybe_tsconfig.value() {
                ProcessedTsconfig::HasPaths(ref tsconfig) => {
                    let resolver =
                        TsconfigPathsResolver::new(tsconfig, &self.node_modules_resolver);
                    tracing::debug!("matched tsconfig {} with \"paths\". resolving against tsconfig resolver (wrapping node_modules)", path.display());

                    let resolution = resolver.resolve(base, module_specifier)?;
                    tracing::debug!(
                        "tsconfig resolved {} to {}",
                        module_specifier,
                        resolution.filename,
                    );

                    Ok(resolution)
                }
                ProcessedTsconfig::NoPaths => {
                    tracing::debug!("matched tsconfig {} with no \"paths\" entry. resolving against node_modules_resolver", path.display());
                    self.node_modules_resolver.resolve(base, module_specifier)
                }
            },
            None => {
                tracing::debug!(
                    "No tsconfig found for {:?}, resolving against node_modules_resolver",
                    base_path
                );
                self.node_modules_resolver.resolve(base, module_specifier)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use pretty_assertions::assert_eq;
    use swc_ecma_loader::TargetEnv;

    fn check_deadlocks() {
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let deadlocks = parking_lot::deadlock::check_deadlock();
            if deadlocks.is_empty() {
                continue;
            }

            tracing::debug!("{} deadlocks detected", deadlocks.len());
            for (i, threads) in deadlocks.iter().enumerate() {
                tracing::debug!("Deadlock #{}", i);
                for t in threads {
                    tracing::debug!("Thread Id {:#?}", t.thread_id());
                    tracing::debug!("{:#?}", t.backtrace());
                }
            }
        });
    }

    #[test]
    pub fn test_bypasses_tsconfig() {
        check_deadlocks();
        let tmp = test_tmpdir!(
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
                        "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
                    }
                }
            }"#,
            "node_modules/to-node-modules/package.json" => r#"{
                "name": "to-node-modules",
                "main": "./index.js"
            }"#,
            "node_modules/to-node-modules/index.js" => r#"export const something = 1;"#
        );

        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            tmp.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );

        let resolution = resolver
            .resolve(
                &FileName::Real(
                    tmp.root()
                        .to_owned()
                        .join("packages/my/importing/module.ts"),
                ),
                "to-node-modules",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    tmp.root()
                        .to_owned()
                        .join("node_modules/to-node-modules/index.js")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_import_literal() {
        let tmp = test_tmpdir!(
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
                        "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
                    }
                }
            }"#,
            "packages/non-glob-specifier/lib/index.ts" => r#"export const something = 1;"#
        );

        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            tmp.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        let resolution = resolver
            .resolve(
                &FileName::Real(tmp.root_join("packages/my/importing/module.ts")),
                "non-glob-specifier",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    tmp.root()
                        .to_owned()
                        .join("packages/non-glob-specifier/lib/index.ts")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_import_glob() {
        let tmp = test_tmpdir!(
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "glob-specifier/lib/*": ["packages/glob-specifier/src/*.ts"],
                        "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
                    }
                }
            }"#,
            "packages/glob-specifier/src/something.ts" => r#"export const something = 1;"#
        );

        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            tmp.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        let resolution = resolver
            .resolve(
                &FileName::Real(tmp.root_join("packages/my/importing/module.ts")),
                "glob-specifier/lib/something",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    tmp.root()
                        .to_owned()
                        .join("packages/glob-specifier/src/something.ts")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_precedence() {
        let tmp = test_tmpdir!(
            "t/tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "glob-specifier/lib/*": ["override-packages/glob-specifier/src/*"]
                    }
                }
            }"#,
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
                        "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
                    }
                }
            }"#,
            "packages/glob-specifier/src/bar.ts" => r#"export const something = 1;"#,
            "t/override-packages/glob-specifier/src/bar.ts" => r#"export const something = 2;"#
        );

        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            tmp.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        let resolution = resolver
            .resolve(
                &FileName::Real(tmp.root_join("t/packages/my/importing/module.ts")),
                "glob-specifier/lib/bar",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    tmp.root()
                        .to_owned()
                        .join("t/override-packages/glob-specifier/src/bar.ts")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_tsconfig_beats_node_modules() {
        let tmp = test_tmpdir!(
            // this tsconfig.json should match the "in-root" rewrite rule
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "in-root/lib/*": ["packages/glob-specifier/src/*"]
                    }
                }
            }"#,
            "root/node_modules/in-root/lib/bar.ts" => r#"export const something = 1;"#,
            "packages/glob-specifier/src/bar.ts" => r#"export const something = 2;"#
        );

        let caches = CombinedResolverCaches::new();
        let resolver = caches.resolver(
            tmp.root(),
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        let resolution = resolver
            .resolve(
                &FileName::Real(tmp.root_join("root/packages/my/importing/module.ts")),
                "in-root/lib/bar.ts",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root()
                        .to_owned()
                        .join("packages/glob-specifier/src/bar.ts")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_no_escape_root() {
        let tmp = test_tmpdir!(
            // this tsconfig.json should match the "in-root" rewrite rule
            "tsconfig.json" => r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "in-root/lib/*": ["packages/glob-specifier/src/*"],
                    }
                }
            }"#,
            "root/node_modules/in-root/lib/bar.ts" => r#"export const something = 1;"#
        );

        let caches = CombinedResolverCaches::new();
        let sub_root = tmp.root_join("root");
        let resolver = caches.resolver(
            &sub_root,
            NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        );
        let resolution = resolver
            .resolve(
                &FileName::Real(tmp.root_join("root/packages/my/importing/module.ts")),
                "in-root/lib/bar.ts",
            )
            .unwrap();

        assert_eq!(
            resolution,
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root()
                        .to_owned()
                        .join("root/node_modules/in-root/lib/bar.ts")
                ),
                slug: None,
            }
        );
    }

    #[test]
    pub fn test_conditional_export_heterogenous() {
        // Test that the resolver can handle a package.json with a mix of
        // single and conditional exports
        let tmp = test_tmpdir!(
            "node_modules/hetero/package.json" => r#"
            {
                "exports": {
                    ".": {
                        "import": "./lib/import-target.ts",
                        "require": "./lib/require-target.ts"
                    },
                    "./foo": "./lib/foo.ts"
                }
            }
            "#,
            "node_modules/hetero/lib/import-target.ts" => r#"export const something = 1;"#,
            "node_modules/hetero/lib/require-target.ts" => r#"export const something = 2;"#,
            "node_modules/hetero/lib/foo.ts" => r#"export const something = 3;"#
        );

        let resolve_with_conditions = |from: &str, to: &str, conditions: Vec<&str>| -> Resolution {
            let caches = CombinedResolverCaches::new();

            // resolve with import
            let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
            options.export_conditions = conditions.into_iter().map(|s| s.to_string()).collect();
            let import_resolver = caches.resolver(tmp.root(), options);
            import_resolver
                .resolve(&FileName::Real(tmp.root().join(from)), to)
                .unwrap()
        };

        // check "resolve" condition
        assert_eq!(
            resolve_with_conditions(
                "root/packages/my/importing/module.ts",
                "hetero",
                vec!["require"],
            ),
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root()
                        .to_owned()
                        .join("node_modules/hetero/lib/require-target.ts")
                ),
                slug: None,
            }
        );

        // check "import" condition
        assert_eq!(
            resolve_with_conditions(
                "root/packages/my/importing/module.ts",
                "hetero",
                vec!["import"],
            ),
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root()
                        .to_owned()
                        .join("node_modules/hetero/lib/import-target.ts")
                ),
                slug: None,
            }
        );

        // check fallback behaviour with multiple conditions
        assert_eq!(
            resolve_with_conditions(
                "packages/my/importing/module.ts",
                "hetero",
                vec!["not-present", "require", "import"],
            ),
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root()
                        .to_owned()
                        .join("node_modules/hetero/lib/require-target.ts")
                ),
                slug: None,
            }
        );

        // check "import" condition and "resolve" condition against a non-conditional export
        assert_eq!(
            resolve_with_conditions(
                "packages/my/importing/module.ts",
                "hetero/foo",
                vec!["require"],
            ),
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root().to_owned().join("node_modules/hetero/lib/foo.ts")
                ),
                slug: None,
            }
        );
        assert_eq!(
            resolve_with_conditions(
                "packages/my/importing/module.ts",
                "hetero/foo",
                vec!["import"],
            ),
            Resolution {
                filename: FileName::Real(
                    // rewrite should not occur (tsconfig is outside the root dir)
                    tmp.root().to_owned().join("node_modules/hetero/lib/foo.ts")
                ),
                slug: None,
            }
        );
    }
}
