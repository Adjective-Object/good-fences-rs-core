//! Modified version of swc_ecma_loader/resolvers/node
//! which is in-turn based on https://github.com/goto-bus-stop/node-resolve
//!
//! See https://github.com/swc-project/swc/blob/f988b66e1fd921266a8abf6fe9bb997b6878e949/crates/swc_ecma_loader/src/resolvers/node.rs

use super::common::AHashMap;
use super::package::{Browser, PackageJson};
use super::pkgjson_exports::ExportedPath;
use super::pkgjson_rewrites::PackageJsonRewriteData;
use super::util;
use super::util::to_absolute_path;
use anyhow::{bail, Context, Error, Result};
use ftree_cache::context_data::{FileContextCache, WithCache};
use path_clean::PathClean;
use pathdiff::diff_paths;
use std::{
    env::current_dir,
    path::{Component, Path, PathBuf},
};
use swc_common::FileName;
use swc_ecma_loader::{
    resolve::{Resolution, Resolve},
    TargetEnv, NODE_BUILTINS,
};
use tracing::{debug, trace, Level};

pub type PackageJsonCacheEntry = WithCache<
    // context defined by package.json files
    PackageJson,
    // each package.json file also has an associated RewriteData cache.
    //
    // The RewriteData cache is used to store derived resolution data for
    // the package.json file, and is lazily populated on-demand.
    Option<PackageJsonRewriteData>,
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

pub(crate) fn is_core_module(s: &str) -> bool {
    NODE_BUILTINS.contains(&s)
}

static DEFAULT_EXPORT_CONDITIONS: &[&str] = &[
    // TODO: make this configurable and drop "source"
    "source", "default", "import", "require",
];

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
    pub fn resolve_as_file(&self, path: &Path) -> Result<Option<PathBuf>, Error> {
        if path
            .to_str()
            .unwrap_or("")
            .contains("getReadWriteRecipientViewStateFromEmailAddress")
        {
            println!("node-resolver resolve_as_file: {:?}", path);
        }

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
    pub fn resolve_as_directory(
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

    /// Resolve a package import (e.g. no package subpath), using the package.json
    /// "main", "browser", and "exports" fields
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

        // Probe the FS or the cache for a package.json file at that path
        let pkg_cache_entry = self.pkg_json_cache.check_dir(pkg_dir).with_context(|| {
            format!(
                "failed to get package.json for directory {:#?}",
                pkg_dir.display(),
            )
        })?;
        let pkg_cache_entry = pkg_cache_entry.value().as_ref();
        // Check if there was a package.json file at the path
        let pkg_with_resolution_cache = match pkg_cache_entry {
            Some(pkg) => pkg,
            // there was no pkg json file to load, so we can't resolve anything.
            None => return Ok(None),
        };
        // Get the cached resolution data data for this package.json file
        let resolution_data = pkg_with_resolution_cache.try_get_cached_or_init(|pkgjson| {
            PackageJsonRewriteData::create(self, pkg_dir, pkgjson)
        })?;
        let mut out = String::new();
        if let Some(exports_map) = resolution_data.exports.as_ref() {
            if let Some(rewritten) = exports_map.rewrite_relative_export(
                ".",
                DEFAULT_EXPORT_CONDITIONS.iter().copied(),
                &mut out,
            )? {
                match rewritten.rewritten_export {
                    ExportedPath::Exported(exported_rel_path) => {
                        let path = pkg_dir.join(exported_rel_path);
                        // there was a resolved export, try to resolve it
                        return self
                            .resolve_as_file(&path)
                            .or_else(|_| self.resolve_as_directory(&path, false));
                    }
                    ExportedPath::Private => {
                        return Err(anyhow::anyhow!(
                            "Index export is marked as private by export '{}' in {}",
                            rewritten.export_kind,
                            pkg_path.display(),
                        ));
                    }
                }
            }
        }

        let pkg = pkg_with_resolution_cache.inner();
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
    fn resolve_node_modules(
        &self,
        // the path to the node_modules directory
        base_dir: &Path,
        // the import target to resolve
        // e.g. something like `react` or `@react/react-dom`
        target: &str,
    ) -> Result<Option<PathBuf>, Error> {
        if self.ignore_node_modules {
            return Ok(None);
        }

        let abs_base = to_absolute_path(base_dir)?;

        let mut iter = self
            .node_modules_cache
            .probe_path_iter(self.monorepo_root, &abs_base);

        loop {
            // loop through the node_modules direcotires
            let (nm_dir_path, nm_dir_entry) = match iter.next() {
                Some(Ok((path, entry))) => (path, entry),
                Some(Err(e)) => return Err(e),
                None => break,
            };

            tracing::debug!("found node_modules directory: {:#?}", nm_dir_path);

            // Use a cached resolution if it exists
            match nm_dir_entry.get_cached().get(target) {
                Some(CachedResolution::Resolution(path)) => {
                    // cached a resolution, return it
                    return Ok(Some(path.clone()));
                }
                Some(CachedResolution::NoResolution) => {
                    // cached a non-resolution, continue iteration onto the next node_modules directory
                    continue;
                }
                None => {}
            };

            // fall through: we have to actually try resolving against this node_modules directory
            // Search for a package.json file to perform package rewrites against
            let (package_name, rel_import) = match util::split_package_import(target) {
                Some((pkg, rest)) => (pkg, rest),
                None => continue,
            };
            let nm_pkg_path = nm_dir_path.join(NODE_MODULES).join(package_name);

            // Get the cached derived data + pkgjson entry for this path.
            let target_file_path: PathBuf = {
                // scope to drop the lock on cached_entry
                let cached_entry: ftree_cache::context_data::CtxRef<
                    Option<WithCache<PackageJson, Option<PackageJsonRewriteData>>>,
                > = self
                    .pkg_json_cache
                    .check_dir(&nm_pkg_path)
                    .with_context(|| {
                        format!(
                            "failed to read package.json for directory {:#?}",
                            nm_pkg_path.display(),
                        )
                    })?;
                let cached_entry_value = cached_entry.value();

                match cached_entry_value {
                    Some(pkg_with_rewrite) => {
                        // check for rewrite data
                        let cached_rewrite_data =
                            pkg_with_rewrite.try_get_cached_or_init(|pkg| {
                                PackageJsonRewriteData::create(self, &nm_pkg_path, pkg)
                            })?;
                        match *cached_rewrite_data {
                            PackageJsonRewriteData {
                                exports: Some(ref rewrite_data),
                                ..
                            } => {
                                // if there is a rewrite data, rewrite the path against it.
                                let mut out = String::new();
                                let rewritten_to = rewrite_data.rewrite_relative_export::<&str>(
                                    &rel_import,
                                    DEFAULT_EXPORT_CONDITIONS.iter().copied(),
                                    &mut out,
                                )?;
                                if let Some(rewritten_to) = rewritten_to {
                                    match rewritten_to.rewritten_export {
                                        ExportedPath::Exported(exported_rel_path) => nm_dir_path
                                            .join(NODE_MODULES)
                                            .join(package_name)
                                            .join(exported_rel_path),
                                        // tried to import a file that is explicitly marked private
                                        ExportedPath::Private => {
                                            return Err(anyhow::anyhow!(
                                            "Import '{target}' is marked as private by export '{}' in {}",
                                            rewritten_to.export_kind,
                                            nm_pkg_path.join(PACKAGE).display(),
                                        ));
                                        }
                                    }
                                } else {
                                    // no matched rewrite, so just use node_modules/imported/path
                                    tracing::debug!(
                                    "no matched rewrite in {:#?} for {rel_import:#?}. Rewrites: {rewrite_data:#?}",
                                    nm_pkg_path.display()
                                );
                                    nm_dir_path.join(NODE_MODULES).join(target)
                                }
                            }
                            _ => {
                                // no rewrite data, so just use node_modules/imported/path
                                tracing::debug!(
                                    "no rewrite data in ${:#?} for {rel_import:#?}",
                                    nm_pkg_path.display()
                                );
                                nm_dir_path.join(NODE_MODULES).join(target)
                            }
                        }
                    }
                    None => {
                        tracing::debug!("no package.json found in: {:#?}", nm_pkg_path);
                        // TODO: fall through to default resolution
                        nm_dir_path.join(NODE_MODULES).join(target)
                    }
                }
            };

            tracing::debug!("attempting resolution: {target_file_path:#?}");
            if let Some(result) = self
                .resolve_as_file(&target_file_path)
                .ok()
                .or_else(|| self.resolve_as_directory(&target_file_path, true).ok())
                .flatten()
            {
                // Resolved! Cache the result and return it.
                nm_dir_entry.get_cached_mut().insert(
                    target.to_string(),
                    CachedResolution::Resolution(result.clone()),
                );
                tracing::debug!("resolved: {target_file_path:#?}");
                return Ok(Some(result));
            } else {
                tracing::debug!("caching failed resolution: {target_file_path:#?}");
                // failed to resolve! Cache the failure so we don't try again,
                // and continue the loop to try the next node_modules directory.
                nm_dir_entry
                    .get_cached_mut()
                    .insert(target.to_string(), CachedResolution::NoResolution);
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
                    if let Some((pkg_path, browser_cache)) =
                        self.pkg_json_cache.probe_path(self.monorepo_root, base)?
                    {
                        let cache_entry_lock = browser_cache.try_get_cached_or_init(|pkgjson| {
                            PackageJsonRewriteData::create(self, pkg_path, pkgjson)
                        })?;

                        let as_abspath = to_absolute_path(path)?;
                        let rewrite = (*cache_entry_lock).rewrite_browser(&as_abspath)?;
                        return self.wrap(Some(rewrite.to_path_buf()));
                    }
                }
            }
            Ok(v)
        });

        file_name
    }
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
