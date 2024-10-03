//! Modified version of swc_ecma_loader/resolvers/node
//! which is in-turn based on https://github.com/goto-bus-stop/node-resolve
//!
//! See https://github.com/swc-project/swc/blob/f988b66e1fd921266a8abf6fe9bb997b6878e949/crates/swc_ecma_loader/src/resolvers/node.rs

use super::node_resolver::CachingNodeModulesResolver;
use anyhow::Result;
use packagejson::{Browser, PackageJson, StringOrBool};
use packagejson_exports::PackageExportRewriteData;
use path_clean::PathClean;
use std::{
    env::current_dir,
    path::{Component, Path, PathBuf},
};
use swc_common::collections::{AHashMap, AHashSet};

// Used to override imports into a package when targeting a Browser environment.
#[derive(Debug, Default)]

pub struct BrowserCache {
    pub rewrites: AHashMap<PathBuf, PathBuf>,
    pub ignores: AHashSet<PathBuf>,
    pub module_rewrites: AHashMap<String, PathBuf>,
    pub module_ignores: AHashSet<String>,
}

// Helper function to compute the browser cache for a package.json file
//
// This will be lazily computed and cached in the PackageJsonCacheEntry as-needed
impl BrowserCache {
    fn create(
        resolver: &CachingNodeModulesResolver,
        pkg_dir: &Path,
        browser: &Browser,
    ) -> Result<Option<BrowserCache>> {
        let map = match &browser {
            Browser::Obj(map) => map,
            _ => return Ok(None),
        };

        let mut bucket = BrowserCache::default();

        for (k, v) in map.iter() {
            let target_key = Path::new(k);
            let mut components = target_key.components();

            // Relative file paths are sources for this package
            let source = if let Some(Component::CurDir) = components.next() {
                let path = pkg_dir.join(k);
                if let Ok(file) = resolver
                    .resolve_as_file(&path)
                    .or_else(|_| resolver.resolve_as_directory(&path, false))
                {
                    file.map(|file| file.clean())
                } else {
                    None
                }
            } else {
                None
            };

            match v {
                StringOrBool::Str(dest) => {
                    let path = pkg_dir.join(dest);
                    let file = resolver
                        .resolve_as_file(&path)
                        .or_else(|_| resolver.resolve_as_directory(&path, false))?;
                    if let Some(file) = file {
                        let target = file.clean();
                        let target = target
                            .strip_prefix(current_dir().unwrap_or_default())
                            .map(|target| target.to_path_buf())
                            .unwrap_or(target);

                        if let Some(source) = source {
                            bucket.rewrites.insert(source, target);
                        } else {
                            bucket.module_rewrites.insert(k.clone(), target);
                        }
                    }
                }
                StringOrBool::Bool(flag) => {
                    // If somebody set boolean `true` which is an
                    // invalid value we will just ignore it
                    if !flag {
                        if let Some(source) = source {
                            bucket.ignores.insert(source);
                        } else {
                            bucket.module_ignores.insert(k.clone());
                        }
                    }
                }
            }
        }

        Ok(Some(bucket))
    }
}

// Derived struct from a package.json file that is used to rewrite
// requests for files within the package.
#[derive(Debug, Default)]
pub struct PackageJsonRewriteData {
    // Browser rewrites to use for cjs requires
    pub browser_cache: Option<BrowserCache>,
    // map of paths to rewrite
    pub exports: Option<PackageExportRewriteData>,
}

// Helper function to compute the rewrite cache for a package.json file
//
// This will be lazily computed and cached in the PackageJsonCacheEntry as-needed
impl PackageJsonRewriteData {
    pub fn create(
        // resolver used to pre-resolve browser rewrites
        resolver: &CachingNodeModulesResolver,
        // path to the root of the package direcory
        pkg_dir: &Path,
        // the package json file
        pkgjson: &PackageJson,
    ) -> Result<Self> {
        let browser_cache = if let Some(browser) = &pkgjson.browser {
            BrowserCache::create(resolver, pkg_dir, browser)?
        } else {
            None
        };

        let exports_rewrite = if let Some(exports) = &pkgjson.exports {
            Some(PackageExportRewriteData::try_from(exports)?)
        } else {
            None
        };

        Ok(Self {
            browser_cache,
            exports: exports_rewrite,
        })
    }

    // Rewrite a path using the browser field if applicable
    pub fn rewrite_browser<'a>(
        &'a self,
        // an absolute path to a file to rewrite
        abs_path: &'a Path,
    ) -> Result<&'a Path> {
        if let Some(browser_cache) = &self.browser_cache {
            if let Some(rewrite) = browser_cache.rewrites.get(abs_path) {
                // resolve the path against the browser field
                if browser_cache.ignores.contains(abs_path) {
                    return Ok(abs_path);
                }
                return Ok(rewrite);
            }
        }

        Ok(abs_path)
    }
}
