//! Modified version of swc_ecma_loader/resolvers/node
//! which is in-turn based on https://github.com/goto-bus-stop/node-resolve
//!
//! See https://github.com/swc-project/swc/blob/f988b66e1fd921266a8abf6fe9bb997b6878e949/crates/swc_ecma_loader/src/resolvers/node.rs

use std::{
    env::current_dir,
    fs::File,
    io::BufReader,
    path::{Component, Path, PathBuf},
};

use super::package::{Browser, PackageJson, StringOrBool};
use anyhow::{bail, Context, Error, Result};
use path_clean::PathClean;
#[cfg(windows)]
use path_clean::PathClean;
use pathdiff::diff_paths;
use swc_common::{
    collections::{AHashMap, AHashSet},
    FileName,
};
use tracing::{debug, trace, Level};

use swc_ecma_loader::{
    resolve::{Resolution, Resolve},
    TargetEnv, NODE_BUILTINS,
};

use super::context_data::{FileContextCache, WithCache};

pub type PackageJsonCacheEntry = WithCache<
    // context defined by package.json files
    PackageJson,
    // each package.json file also has an associated BrowserCache
    //
    // This is a cache of resolved browser fields for a package.json file
    //
    // This is a double-nested Option because the value is lazily populated
    // and also optional. The outer Option is None if the lazy population
    // has not yet occurred, and the inner Option is None if the package.json
    // file does not have a "browser" field.
    Option<Option<BrowserCache>>,
>;

pub type PackageJsonCache = FileContextCache<PackageJsonCacheEntry, "package.json">;

const NODE_MODULES: &str = "node_modules";

pub type NodeModulesCache = FileContextCache<
    WithCache<
        // node_modules directories don't actually have any context data
        // since there's not a config file to parse
        (),
        // Track resolution of paths into each node_modules directory, so
        // we can optimisitcally resolve paths without hitting the filesystem
        AHashMap<String, CachedResolution>,
    >,
    NODE_MODULES,
>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachedResolution {
    // Some(path) means the path was resolved to a file
    // None means the path was resolved to a directory
    Resolution(PathBuf),
    NoResolution,
}

static PACKAGE: &str = "package.json";

#[derive(Debug, Default)]
pub struct BrowserCache {
    rewrites: AHashMap<PathBuf, PathBuf>,
    ignores: AHashSet<PathBuf>,
    module_rewrites: AHashMap<String, PathBuf>,
    module_ignores: AHashSet<String>,
}

pub fn to_absolute_path(path: &Path) -> Result<PathBuf, Error> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir()?.join(path)
    }
    .clean();

    Ok(absolute_path)
}

pub(crate) fn is_core_module(s: &str) -> bool {
    NODE_BUILTINS.contains(&s)
}

// Adaptation of NodeModulesResolver from swc that stores
// intermediate data in shared caches instead of going to disk
// for every resolution.
//
// Lifted from swc_ecma_loader-0.45.23/src/resolvers/node.rs
pub struct CachingNodeModulesResolver<'a> {
    monorepo_root: &'a Path,
    target_env: TargetEnv,
    alias: AHashMap<String, String>,
    // if true do not resolve symlink
    preserve_symlinks: bool,
    ignore_node_modules: bool,

    // cache of package.json files discovered during resolution
    pkg_json_cache: &'a PackageJsonCache,

    // cache of node_modules directories discovered during resolution
    node_modules_cache: &'a NodeModulesCache,
}

static EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "json", "node"];

impl<'caches> CachingNodeModulesResolver<'caches> {
    /// Create a node modules resolver for the target runtime environment.
    pub fn new(
        target_env: TargetEnv,
        alias: AHashMap<String, String>,
        preserve_symlinks: bool,
        monorepo_root: &'caches Path,
        pkg_json_cache: &'caches PackageJsonCache,
        node_modules_cache: &'caches NodeModulesCache,
    ) -> Self {
        Self {
            target_env,
            alias,
            preserve_symlinks,
            monorepo_root,
            ignore_node_modules: false,
            pkg_json_cache,
            node_modules_cache,
        }
    }

    fn wrap(&self, path: Option<PathBuf>) -> Result<FileName, Error> {
        if let Some(path) = path {
            if self.preserve_symlinks {
                return Ok(FileName::Real(path.clean()));
            } else {
                return Ok(FileName::Real(path.canonicalize()?));
            }
        }
        bail!("index not found")
    }

    /// Resolve a path as a file. If `path` refers to a file, it is returned;
    /// otherwise the `path` + each extension is tried.
    fn resolve_as_file(&self, path: &Path) -> Result<Option<PathBuf>, Error> {
        let _tracing = if cfg!(debug_assertions) {
            Some(
                tracing::span!(
                    Level::ERROR,
                    "resolve_as_file",
                    path = tracing::field::display(path.display())
                )
                .entered(),
            )
        } else {
            None
        };

        if cfg!(debug_assertions) {
            trace!("resolve_as_file({})", path.display());
        }

        let try_exact = path.extension().is_some();
        if try_exact {
            if path.is_file() {
                return Ok(Some(path.to_path_buf()));
            }
        } else {
            // We try `.js` first.
            let mut path = path.to_path_buf();
            path.set_extension("js");
            if path.is_file() {
                return Ok(Some(path));
            }
        }

        // Try exact file after checking .js, for performance
        if !try_exact && path.is_file() {
            return Ok(Some(path.to_path_buf()));
        }

        if let Some(name) = path.file_name() {
            let mut ext_path = path.to_path_buf();
            let name = name.to_string_lossy();
            for ext in EXTENSIONS {
                ext_path.set_file_name(format!("{}.{}", name, ext));
                if ext_path.is_file() {
                    return Ok(Some(ext_path));
                }
            }

            // TypeScript-specific behavior: if the extension is ".js" or ".jsx",
            // try replacing it with ".ts" or ".tsx".
            ext_path.set_file_name(name.into_owned());
            let old_ext = path.extension().and_then(|ext| ext.to_str());

            if let Some(old_ext) = old_ext {
                let extensions: &[&str] = match old_ext {
                    // Note that the official compiler code always tries ".ts" before
                    // ".tsx" even if the original extension was ".jsx".
                    "js" => &["ts", "tsx"],
                    "jsx" => &["ts", "tsx"],
                    "mjs" => &["mts"],
                    "cjs" => &["cts"],
                    _ => &[],
                };

                for ext in extensions {
                    ext_path.set_extension(ext);

                    if ext_path.is_file() {
                        return Ok(Some(ext_path));
                    }
                }
            }
        }

        bail!("file not found: {}", path.display())
    }

    /// Resolve a path as a directory, using the "main" key from a package.json
    /// file if it exists, or resolving to the index.EXT file if it exists.
    fn resolve_as_directory(
        &self,
        path: &Path,
        allow_package_entry: bool,
    ) -> Result<Option<PathBuf>, Error> {
        let _tracing = if cfg!(debug_assertions) {
            Some(
                tracing::span!(
                    Level::ERROR,
                    "resolve_as_directory",
                    path = tracing::field::display(path.display())
                )
                .entered(),
            )
        } else {
            None
        };

        if cfg!(debug_assertions) {
            trace!("resolve_as_directory({})", path.display());
        }

        // TODO use pkgjson cache here
        let pkg_path = path.join(PACKAGE);
        if allow_package_entry && pkg_path.is_file() {
            if let Some(main) = self.resolve_package_entry(path, &pkg_path)? {
                return Ok(Some(main));
            }
        }

        // Try to resolve to an index file.
        for ext in EXTENSIONS {
            let ext_path = path.join(format!("index.{}", ext));
            if ext_path.is_file() {
                return Ok(Some(ext_path));
            }
        }
        Ok(None)
    }

    /// Resolve using the package.json "main" or "browser" keys.
    fn resolve_package_entry(
        &self,
        pkg_dir: &Path,
        pkg_path: &Path,
    ) -> Result<Option<PathBuf>, Error> {
        let _tracing = if cfg!(debug_assertions) {
            Some(
                tracing::span!(
                    Level::ERROR,
                    "resolve_package_entry",
                    pkg_dir = tracing::field::display(pkg_dir.display()),
                    pkg_path = tracing::field::display(pkg_path.display()),
                )
                .entered(),
            )
        } else {
            None
        };

        // TODO: use pkgjson cache here
        let file = File::open(pkg_path)?;
        let reader = BufReader::new(file);
        let pkg: PackageJson = serde_json::from_reader(reader)
            .context(format!("failed to deserialize {}", pkg_path.display()))?;
        // TODO: parse pkgjson exports field here

        let main_fields = match self.target_env {
            TargetEnv::Node => {
                vec![pkg.module.as_ref(), pkg.main.as_ref()]
            }
            TargetEnv::Browser => {
                if let Some(browser) = &pkg.browser {
                    match browser {
                        Browser::Str(path) => {
                            vec![Some(path), pkg.module.as_ref(), pkg.main.as_ref()]
                        }
                        Browser::Obj(_) => {
                            vec![pkg.module.as_ref(), pkg.main.as_ref()]
                        }
                    }
                } else {
                    vec![pkg.module.as_ref(), pkg.main.as_ref()]
                }
            }
        };

        if let Some(Some(target)) = main_fields.iter().find(|x| x.is_some()) {
            let path = pkg_dir.join(target);
            return self
                .resolve_as_file(&path)
                .or_else(|_| self.resolve_as_directory(&path, false));
        }

        Ok(None)
    }

    /// Resolve by walking up node_modules folders.
    fn resolve_node_modules<'a>(
        &'a self,
        base_dir: &Path,
        target: &str,
    ) -> Result<Option<PathBuf>, Error> {
        if self.ignore_node_modules {
            return Ok(None);
        }

        let abs_base = to_absolute_path(base_dir)?;

        let mut iter = self
            .node_modules_cache
            .probe_path_iter(&self.monorepo_root, &abs_base);

        loop {
            let (nm_dir_path, nm_dir_entry) = match iter.next() {
                Some(Ok((path, entry))) => (path, entry),
                Some(Err(e)) => return Err(e),
                None => break,
            };

            let cached_resolution: Option<CachedResolution> = {
                let cached = nm_dir_entry.get_cached();
                cached.get(target).map(|v| v.clone())
            };

            match cached_resolution {
                Some(CachedResolution::Resolution(path)) => {
                    // cached a resolution, return it
                    return Ok(Some(path.clone()));
                }
                Some(CachedResolution::NoResolution) => {
                    // cached a non-resolution, continue iteration onto the next node_modules directory
                    continue;
                }
                None => {
                    // no cache hit for this resolution against this nm_directory.
                    //
                    // resolve it, then cache the result
                    let path = nm_dir_path.join(NODE_MODULES).join(target);
                    if let Some(result) = self
                        .resolve_as_file(&path)
                        .ok()
                        .or_else(|| self.resolve_as_directory(&path, true).ok())
                        .flatten()
                    {
                        nm_dir_entry.get_cached_mut().insert(
                            target.to_string(),
                            CachedResolution::Resolution(result.clone()),
                        );

                        return Ok(Some(result));
                    } else {
                        nm_dir_entry
                            .get_cached_mut()
                            .insert(target.to_string(), CachedResolution::NoResolution);
                    }
                }
            }
        }

        // no matches anywhere, so this failed to resolve against any node_modules directory.
        Ok(None)
    }

    fn resolve_filename(&self, base: &FileName, module_specifier: &str) -> Result<FileName, Error> {
        debug!(
            "Resolving {} from {:#?} for {:#?}",
            module_specifier, base, self.target_env
        );

        if !module_specifier.starts_with('.') {
            // Handle absolute path

            let path = Path::new(module_specifier);

            if let Ok(file) = self
                .resolve_as_file(path)
                .or_else(|_| self.resolve_as_directory(path, false))
            {
                if let Ok(file) = self.wrap(file) {
                    return Ok(file);
                }
            }
        }

        let base = match base {
            FileName::Real(v) => v,
            _ => bail!("node-resolver supports only files"),
        };

        let base_dir = if base.is_file() {
            let cwd = &Path::new(".");
            base.parent().unwrap_or(cwd)
        } else {
            base
        };

        // Handle builtin modules for nodejs
        if let TargetEnv::Node = self.target_env {
            if module_specifier.starts_with("node:") {
                return Ok(FileName::Custom(module_specifier.into()));
            }

            if is_core_module(module_specifier) {
                return Ok(FileName::Custom(format!("node:{}", module_specifier)));
            }
        }

        // Aliases allow browser shims to be renamed so we can
        // map `stream` to `stream-browserify` for example
        let target = if let Some(alias) = self.alias.get(module_specifier) {
            &alias[..]
        } else {
            module_specifier
        };

        let target_path = Path::new(target);

        let file_name = {
            if target_path.is_absolute() {
                let path = PathBuf::from(target_path);
                self.resolve_as_file(&path)
                    .or_else(|_| self.resolve_as_directory(&path, true))
                    .and_then(|p| self.wrap(p))
            } else {
                let mut components = target_path.components();

                if let Some(Component::CurDir | Component::ParentDir) = components.next() {
                    #[cfg(windows)]
                    let path = {
                        let base_dir = BasePath::new(base_dir).unwrap();
                        base_dir
                            .join(target.replace('/', "\\"))
                            .normalize_virtually()
                            .unwrap()
                            .into_path_buf()
                    };
                    #[cfg(not(windows))]
                    let path = base_dir.join(target);
                    self.resolve_as_file(&path)
                        .or_else(|_| self.resolve_as_directory(&path, true))
                        .and_then(|p| self.wrap(p))
                } else {
                    self.resolve_node_modules(base_dir, target)
                        .and_then(|path| {
                            let file_path = path.context("failed to get the node_modules path");
                            let current_directory = current_dir()?;
                            let relative_path = diff_paths(file_path?, current_directory);
                            self.wrap(relative_path)
                        })
                }
            }
        }
        .and_then(|v| {
            // Handle path references for the `browser` package config
            if let TargetEnv::Browser = self.target_env {
                if let FileName::Real(path) = &v {
                    // probe for a package.json file
                    if let Some((_, browser_cache)) =
                        self.pkg_json_cache.probe_path(self.monorepo_root, base)?
                    {
                        // once we find a package.json file, see if it contains a browser field
                        if let Some(ref pkgjson_browser_cache) = *(browser_cache
                            .try_get_cached_or_init(
                                (self, base_dir),
                                compute_pkgjson_browsercache,
                            )?)
                        {
                            // resolve the path against the browser field
                            let path = to_absolute_path(path).unwrap();
                            if pkgjson_browser_cache.ignores.contains(&path) {
                                return Ok(FileName::Custom(path.display().to_string()));
                            }
                            if let Some(rewrite) = pkgjson_browser_cache.rewrites.get(&path) {
                                return self.wrap(Some(rewrite.to_path_buf()));
                            }
                        }
                    }
                }
            }
            Ok(v)
        });

        file_name
    }
}

// Helper function to compute the browser cache for a package.json file
//
// This will be lazily computed and cached in the PackageJsonCacheEntry as-needed
fn compute_pkgjson_browsercache(
    (resolver, pkg_dir): (&CachingNodeModulesResolver, &Path),
    pkgjson: &PackageJson,
) -> Result<Option<BrowserCache>> {
    let map = match &pkgjson.browser {
        Some(Browser::Obj(map)) => map,
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

    return Ok(Some(bucket));
}

impl<'a> Resolve for CachingNodeModulesResolver<'a> {
    fn resolve(&self, base: &FileName, module_specifier: &str) -> Result<Resolution, Error> {
        self.resolve_filename(base, module_specifier)
            .map(|filename| Resolution {
                filename,
                slug: None,
            })
    }
}
