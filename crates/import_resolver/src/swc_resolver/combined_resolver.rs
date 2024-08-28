use anyhow::{anyhow, Error};
use std::path::Path;
use swc_common::{collections::AHashMap, FileName};
use swc_core::ecma::loader::resolve::Resolution;
use swc_ecma_loader::{resolve::Resolve, TargetEnv};

use super::{
    node_resolver::{CachingNodeModulesResolver, NodeModulesCache, PackageJsonCache},
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
        target_env: TargetEnv,
        alias: AHashMap<String, String>,
        preserve_symlinks: bool,
    ) -> CombinedResolver<'a> {
        return CombinedResolver::<'a> {
            root_dir,
            tsconfig_cache: &self.tsconfig_cache,
            node_modules_resolver: CachingNodeModulesResolver::new(
                target_env,
                alias,
                preserve_symlinks,
                root_dir,
                &self.package_json_cache,
                &self.node_modules_cache,
            ),
        };
    }

    /// Mark the files files in the given path as dirty
    pub fn mark_dirty_root(&mut self, root: &Path) {
        self.tsconfig_cache.mark_dirty_root(root);
        self.node_modules_cache.mark_dirty_root(root);
        self.package_json_cache.mark_dirty_root(root);
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
            Some((_, maybe_tsconfig)) => match maybe_tsconfig.value() {
                ProcessedTsconfig::HasPaths(ref tsconfig) => {
                    let resolver =
                        TsconfigPathsResolver::new(tsconfig, &self.node_modules_resolver);

                    resolver.resolve(base, module_specifier)
                }
                ProcessedTsconfig::NoPaths => {
                    self.node_modules_resolver.resolve(base, module_specifier)
                }
            },
            None => self.node_modules_resolver.resolve(base, module_specifier),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use pretty_assertions::assert_eq;
    use test_tmpdir::TmpDir;
    // use tracing_test::traced_test;

    fn check_deadlocks() {
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let deadlocks = parking_lot::deadlock::check_deadlock();
            if deadlocks.is_empty() {
                continue;
            }

            println!("{} deadlocks detected", deadlocks.len());
            for (i, threads) in deadlocks.iter().enumerate() {
                println!("Deadlock #{}", i);
                for t in threads {
                    println!("Thread Id {:#?}", t.thread_id());
                    println!("{:#?}", t.backtrace());
                }
            }
        });
    }

    #[test]
    // #[traced_test]
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
        let resolver = caches.resolver(tmp.root(), TargetEnv::Node, Default::default(), false);

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
                        .join("node_modules/to-node-modules/index.ts")
                ),
                slug: None,
            }
        );
    }

    // #[test]
    // pub fn test_import_literal() {
    //     let tmp = test_tmpdir!(
    //         "tsconfig.json" => r#"{
    //             "compilerOptions": {
    //                 "baseUrl": ".",
    //                 "paths": {
    //                     "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
    //                     "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
    //                 }
    //             }
    //         }"#,
    //         "packages/non-glob-specifier/lib/index.ts" => r#"export const something = 1;"#
    //     );

    //     let expected_resolver = &TestResolver::new(
    //         tmp.root(), // the base path comes from the tsconfig's base_path in tsconfig resolver
    //         "./packages/non-glob-specifier/lib/index",
    //     );
    //     let factory = CombinedTsconfigResolver::new(tmp.root());
    //     let resolver = factory.resolver(expected_resolver);
    //     let resolution = resolver
    //         .resolve(
    //             &FileName::Real(tmp.root_join("packages/my/importing/module.ts")),
    //             "non-glob-specifier",
    //         )
    //         .unwrap();
    //     expected_resolver.assert(resolution);
    // }

    // #[test]
    // pub fn test_import_glob() {
    //     let tmp = test_tmpdir!(
    //         "tsconfig.json" => r#"{
    //             "compilerOptions": {
    //                 "baseUrl": ".",
    //                 "paths": {
    //                     "glob-specifier/lib/*": ["packages/glob-specifier/src/*.ts"],
    //                     "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
    //                 }
    //             }
    //         }"#,
    //         "packages/glob-specifier/src/something.ts" => r#"export const something = 1;"#
    //     );

    //     let expected_resolver = &TestResolver::new(
    //         tmp.root(), // the base path comes from the tsconfig's base_path in tsconfig resolver
    //         "./packages/glob-specifier/src/something.ts",
    //     );
    //     let factory = CombinedTsconfigResolver::new(tmp.root());
    //     let resolver = factory.resolver(expected_resolver);
    //     let resolution = resolver
    //         .resolve(
    //             &FileName::Real(tmp.root_join("packages/my/importing/module.ts")),
    //             "glob-specifier/lib/something",
    //         )
    //         .unwrap();
    //     expected_resolver.assert(resolution);
    // }

    // #[test]
    // pub fn test_precedence() {
    //     let tmp = test_tmpdir!(
    //         "t/tsconfig.json" => r#"{
    //             "compilerOptions": {
    //                 "baseUrl": ".",
    //                 "paths": {
    //                     "glob-specifier/lib/*": ["override-packages/glob-specifier/src/*"]
    //                 }
    //             }
    //         }"#,
    //         "tsconfig.json" => r#"{
    //             "compilerOptions": {
    //                 "baseUrl": ".",
    //                 "paths": {
    //                     "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
    //                     "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
    //                 }
    //             }
    //         }"#
    //     );

    //     let expected_resolver = &TestResolver::new(
    //         tmp.root_join("t"),
    //         "./override-packages/glob-specifier/src/bar",
    //     );
    //     let factory = CombinedTsconfigResolver::new(tmp.root());
    //     let resolver = factory.resolver(expected_resolver);
    //     let resolution = resolver
    //         .resolve(
    //             &FileName::Real(tmp.root_join("t/packages/my/importing/module.ts")),
    //             "glob-specifier/lib/bar",
    //         )
    //         .unwrap();
    //     expected_resolver.assert(resolution)
    // }

    // #[test]
    // pub fn test_no_escape_root() {
    //     let tmp = test_tmpdir!(
    //         "tsconfig.json" => r#"{
    //             "compilerOptions": {
    //                 "baseUrl": ".",
    //                 "paths": {
    //                     "glob-specifier/lib/*": ["packages/glob-specifier/src/*"],
    //                     "non-glob-specifier": ["packages/non-glob-specifier/lib/index"]
    //                 }
    //             }
    //         }"#
    //     );

    //     let expected_resolver = &TestResolver::new(
    //         tmp.root_join("root/packages/my/importing/module.ts"),
    //         "glob-specifier/lib/bar",
    //     );
    //     let factory = CombinedTsconfigResolver::new(tmp.root_join("root").as_path());
    //     let resolver = factory.resolver(expected_resolver);
    //     let resolution = resolver
    //         .resolve(
    //             &FileName::Real(tmp.root_join("root/packages/my/importing/module.ts")),
    //             "glob-specifier/lib/bar",
    //         )
    //         .unwrap();
    //     expected_resolver.assert(resolution)
    // }
}
