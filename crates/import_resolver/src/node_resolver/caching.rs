//! Modified version of swc_ecma_loader/resolvers/node
//! which is in-turn based on https://github.com/goto-bus-stop/node-resolve
//!
//! See https://github.com/swc-project/swc/blob/f988b66e1fd921266a8abf6fe9bb997b6878e949/crates/swc_ecma_loader/src/resolvers/node.rs

use super::common::AHashMap;
use super::pkgjson_rewrites::PackageJsonRewriteData;
use super::util;
use crate::resolve::{PathResolver, Resolution, TargetEnv, NODE_BUILTINS};
use abspath::join_abspath;
use anyhow::{bail, Context, Error, Result};
use ftree_cache::context_data::{FileContextCache, WithCache};
use ftree_cache::inputs::{InputObserver, InputProof};
use packagejson::exported_path::ExportedPathRef;
use packagejson::{package_relative_path, Browser, PackageJson};
use path_clean::PathClean;
use pathdiff::diff_paths;
use std::{
    env::current_dir,
    path::{Component, Path, PathBuf},
    sync::Arc,
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
        AHashMap<
            (String, Vec<String>, Vec<String>, bool, TargetEnv, bool),
            Arc<parking_lot::Mutex<Option<(CachedResolution, InputProof)>>>,
        >,
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

    // Active export conditions; package declaration order determines priority.
    export_conditions: Vec<String>,

    // list of extensions to use when resolving files
    extensions: Vec<String>,
    prefer_js_extension: bool,
    pub(crate) observer: InputObserver,
}

pub const DEFAULT_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "json", "node", "jsonc"];
pub const DEFAULT_EXPORT_CODITIONS: &[&str] = &[
    "node",
    "node-addons",
    "module-sync",
    "import",
    "require",
    "default",
];

#[derive(Clone)]
pub struct NodeModulesResolverOptions {
    pub target_env: TargetEnv,
    pub alias: AHashMap<String, String>,
    pub preserve_symlinks: bool,
    pub ignore_node_modules: bool,
    pub extensions: Vec<String>,
    /// Preserve legacy `.js` probing before exact extensionless files and suffixes.
    /// Set false to use only the ordered `extensions` list after exact files.
    pub prefer_js_extension: bool,
    pub export_conditions: Vec<String>,
}

impl NodeModulesResolverOptions {
    pub fn default_for_env(target_env: TargetEnv) -> Self {
        Self {
            target_env,
            alias: AHashMap::default(),
            preserve_symlinks: false,
            ignore_node_modules: false,
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
            prefer_js_extension: true,
            export_conditions: DEFAULT_EXPORT_CODITIONS
                .iter()
                .copied()
                .filter(|condition| {
                    target_env == TargetEnv::Node
                        || !matches!(*condition, "node" | "node-addons" | "module-sync")
                })
                .chain((target_env == TargetEnv::Browser).then_some("browser"))
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl<'caches> CachingNodeModulesResolver<'caches> {
    /// Create a node modules resolver for the target runtime environment.
    pub fn new(
        monorepo_root: &'caches Path,
        pkg_json_cache: &'caches PackageJsonCache,
        node_modules_cache: &'caches NodeModulesCache,
        options: NodeModulesResolverOptions,
    ) -> Self {
        Self {
            monorepo_root,
            pkg_json_cache,
            node_modules_cache,
            // unpack options
            target_env: options.target_env,
            alias: options.alias,
            preserve_symlinks: options.preserve_symlinks,
            ignore_node_modules: options.ignore_node_modules,
            extensions: options.extensions,
            prefer_js_extension: options.prefer_js_extension,
            export_conditions: options.export_conditions,
            observer: InputObserver::default(),
        }
    }

    pub(crate) fn with_observer(mut self, observer: InputObserver) -> Self {
        self.observer = observer;
        self
    }

    fn wrap(&self, path: Option<PathBuf>) -> Result<PathBuf, Error> {
        if let Some(path) = path {
            match self.observer.capture(&path) {
                Ok(input) => {
                    let canonical = input.canonical().map(Path::to_owned);
                    self.observer.record(input);
                    if self.preserve_symlinks {
                        Ok(path.clean())
                    } else {
                        canonical.ok_or_else(|| {
                            anyhow::anyhow!("resolved file no longer exists: {}", path.display())
                        })
                    }
                }
                Err(error) => {
                    self.observer.error(&error);
                    Err(Error::from_boxed(error.into()))
                }
            }
        } else {
            Err(anyhow::anyhow!("file not found"))
        }
    }

    /// Resolve a path as a file. If `path` refers to a file, it is returned;
    /// otherwise the `path` + each extension is tried.
    pub fn resolve_as_file(&self, path: &Path) -> Result<Option<PathBuf>, Error> {
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
            if self.observer.is_file(path) {
                return Ok(Some(path.to_path_buf()));
            }
        } else if self.prefer_js_extension {
            // We try `.js` first.
            let mut path = path.to_path_buf();
            path.set_extension("js");
            if self.observer.is_file(&path) {
                return Ok(Some(path));
            }
        }

        // Legacy policy probes .js before an exact extensionless file.
        if !try_exact && self.observer.is_file(path) {
            return Ok(Some(path.to_path_buf()));
        }

        if let Some(name) = path.file_name() {
            let mut ext_path = path.to_path_buf();
            let name = name.to_string_lossy();
            for ext in self.extensions.iter() {
                ext_path.set_file_name(format!("{}.{}", name, ext));
                if self.observer.is_file(&ext_path) {
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
                    "mjsx" => &["mtsx"],
                    "cjsx" => &["ctsx"],
                    _ => &[],
                };

                for ext in extensions {
                    ext_path.set_extension(ext);

                    if self.observer.is_file(&ext_path) {
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
        if cfg!(debug_assertions) {
            trace!("resolve_as_directory({})", path.display());
        }

        // TODO use pkgjson cache here
        let pkg_path = path.join(PACKAGE);
        if allow_package_entry && self.observer.is_file(&pkg_path) {
            if let Some(main) = self.resolve_package_entry(path, &pkg_path)? {
                return Ok(Some(main));
            }
        }

        // Try to resolve to an index file.
        for ext in self.extensions.iter() {
            let ext_path = path.join(format!("index.{}", ext));
            if self.observer.is_file(&ext_path) {
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
        let pkg_cache_entry = self
            .pkg_json_cache
            .check_dir_with_inputs(pkg_dir, Some(&self.observer))
            .with_context(|| {
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
        self.observer.replay(&resolution_data.inputs);
        let mut out = String::new();
        if let Some(exports_map) = resolution_data.exports.as_ref() {
            if let Some(rewritten) = exports_map.rewrite_relative_export(
                ".",
                self.export_conditions.iter().map(|s| s.as_str()),
                &mut out,
            )? {
                match rewritten.rewritten_export {
                    ExportedPathRef::Exported(exported_rel_path) => {
                        let path = pkg_dir.join(package_relative_path(exported_rel_path).as_ref());
                        // there was a resolved export, try to resolve it
                        return self
                            .resolve_as_file(&path)
                            .or_else(|_| self.resolve_as_directory(&path, false));
                    }
                    ExportedPathRef::Private => {
                        return Err(anyhow::anyhow!(
                            "Index export is marked as private by export '{}' in {}",
                            rewritten.export_kind,
                            pkg_path.display(),
                        ));
                    }
                    ExportedPathRef::Unrecognized
                    | ExportedPathRef::Conditional(_)
                    | ExportedPathRef::Fallback(_) => {
                        return Err(anyhow::anyhow!(
                            "Index export had an unrecognized format and will be treated as private during resolution",
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
            let path = pkg_dir.join(package_relative_path(target).as_ref());
            let resolved = self
                .resolve_as_file(&path)
                .or_else(|_| self.resolve_as_directory(&path, false))?;
            return resolved
                .map(|path| {
                    if self.target_env == TargetEnv::Browser {
                        let absolute = join_abspath(self.monorepo_root, &path)?;
                        let rewritten = resolution_data.rewrite_browser(&absolute)?;
                        // BrowserCache destinations can be cwd-relative, unlike package entries.
                        if rewritten.is_relative() {
                            Ok(current_dir()?.join(rewritten))
                        } else {
                            Ok(rewritten.to_path_buf())
                        }
                    } else {
                        Ok(path)
                    }
                })
                .transpose();
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

        let mut conditions = self.export_conditions.clone();
        conditions.sort();
        conditions.dedup();
        let key = (
            target.to_owned(),
            conditions,
            self.extensions.clone(),
            self.prefer_js_extension,
            self.target_env,
            self.preserve_symlinks,
        );
        let abs_base = join_abspath(self.monorepo_root, base_dir)?;

        let mut iter = self.node_modules_cache.probe_path_iter_with_inputs(
            self.monorepo_root,
            &abs_base,
            &self.observer,
        );

        loop {
            // loop through the node_modules directories
            let (nm_dir_path, nm_dir_entry) = match iter.next() {
                Some(Ok((path, entry))) => (path, entry),
                Some(Err(e)) => return Err(e),
                None => break,
            };

            tracing::debug!("found node_modules directory: {:#?}", nm_dir_path);

            let slot = nm_dir_entry
                .get_cached_mut()
                .entry(key.clone())
                .or_default()
                .clone();
            let mut cached = slot.lock();
            match cached.as_ref() {
                Some((CachedResolution::Resolution(path), inputs)) => {
                    self.observer.replay(inputs);
                    // cached a resolution, return it
                    return Ok(Some(path.clone()));
                }
                Some((CachedResolution::NoResolution, inputs)) => {
                    self.observer.replay(inputs);
                    // cached a non-resolution, continue iteration onto the next node_modules directory
                    continue;
                }
                None => {}
            };

            let input_start = self.observer.position();
            let result = self.resolve_in_node_modules(nm_dir_path, target)?;
            *cached = Some((
                result
                    .as_ref()
                    .map_or(CachedResolution::NoResolution, |path| {
                        CachedResolution::Resolution(path.clone())
                    }),
                self.observer.since(input_start),
            ));
            if result.is_some() {
                return Ok(result);
            }
        }

        Ok(None)
    }

    fn resolve_in_node_modules(
        &self,
        nm_dir_path: &Path,
        target: &str,
    ) -> Result<Option<PathBuf>, Error> {
        let (package_name, rel_import) = match util::split_package_import(target) {
            Some((pkg, rest)) => (pkg, rest),
            None => return Ok(None),
        };
        let nm_pkg_path = nm_dir_path.join(NODE_MODULES).join(package_name);

        // Get the cached derived data + pkgjson entry for this path.
        let target_file_path: PathBuf = {
            // scope to drop the lock on cached_entry
            let cached_entry: ftree_cache::context_data::CtxRef<
                Option<WithCache<PackageJson, Option<PackageJsonRewriteData>>>,
            > = self
                .pkg_json_cache
                .check_dir_with_inputs(&nm_pkg_path, Some(&self.observer))
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
                    let cached_rewrite_data = pkg_with_rewrite.try_get_cached_or_init(|pkg| {
                        PackageJsonRewriteData::create(self, &nm_pkg_path, pkg)
                    })?;
                    self.observer.replay(&cached_rewrite_data.inputs);
                    match *cached_rewrite_data {
                        PackageJsonRewriteData {
                            exports: Some(ref rewrite_data),
                            ..
                        } => {
                            // if there is a rewrite data, rewrite the path against it.
                            let mut out = String::new();
                            let rewritten_to = rewrite_data.rewrite_relative_export::<&str>(
                                &rel_import,
                                self.export_conditions.iter().map(|s| s.as_str()),
                                &mut out,
                            )?;
                            if let Some(rewritten_to) = rewritten_to {
                                match rewritten_to.rewritten_export {
                                    ExportedPathRef::Exported(exported_rel_path) => nm_pkg_path
                                        .join(package_relative_path(exported_rel_path).as_ref()),
                                    // tried to import a file that is explicitly marked private
                                    ExportedPathRef::Private => {
                                        return Err(anyhow::anyhow!(
                                                "Import '{target}' is marked as private by export '{}' in {}",
                                                rewritten_to.export_kind,
                                                nm_pkg_path.join(PACKAGE).display(),
                                            ));
                                    }
                                    ExportedPathRef::Unrecognized
                                    | ExportedPathRef::Conditional(_)
                                    | ExportedPathRef::Fallback(_) => {
                                        return Err(anyhow::anyhow!(
                                                "Import '{target}' matched export '{}' in {}, which has an unrecognized export format. It will be treated as private",
                                                rewritten_to.export_kind,
                                                nm_pkg_path.join(PACKAGE).display(),
                                            ));
                                    }
                                }
                            } else {
                                return Err(Error::from_boxed(
                                        eyre::eyre!(
                                            "ERR_PACKAGE_PATH_NOT_EXPORTED: import {target:?} is not exported by {}",
                                            nm_pkg_path.join(PACKAGE).display()
                                        )
                                        .into(),
                                    ));
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
            tracing::debug!("resolved: {target_file_path:#?}");
            return Ok(Some(result));
        }

        Ok(None)
    }

    fn resolve_package_import(
        &self,
        base_dir: &Path,
        request: &str,
        depth: usize,
    ) -> Result<PathBuf> {
        let result = (|| {
            if request == "#" || request.starts_with("#/") {
                return Err(Error::from_boxed(
                    eyre::eyre!("ERR_INVALID_MODULE_SPECIFIER: invalid package import {request:?}")
                        .into(),
                ));
            }
            if depth >= 64 {
                return Err(Error::from_boxed(
                    eyre::eyre!("ERR_PACKAGE_IMPORT_CYCLE: package import recursion limit reached")
                        .into(),
                ));
            }
            let absolute = join_abspath(self.monorepo_root, base_dir)?;
            for directory in absolute.ancestors() {
                if directory
                    .file_name()
                    .is_some_and(|name| name == NODE_MODULES)
                {
                    break;
                }
                let entry = self
                    .pkg_json_cache
                    .check_dir_with_inputs(directory, Some(&self.observer))
                    .with_context(|| {
                        format!(
                            "reading package import scope manifest {}",
                            directory.join(PACKAGE).display()
                        )
                    })?;
                if let Some(package) = entry.value().as_ref() {
                    let imports = package.inner().imports.as_ref().with_context(|| {
                        format!(
                            "ERR_PACKAGE_IMPORT_NOT_DEFINED: {request:?} in {}",
                            directory.join(PACKAGE).display()
                        )
                    })?;
                    let target = packagejson_exports::rewrite_package_import(
                        imports,
                        request,
                        self.export_conditions.iter().map(String::as_str),
                    )
                    .with_context(|| {
                        format!(
                            "selecting import {request:?} in {}",
                            directory.join(PACKAGE).display()
                        )
                    })?;
                    if target.starts_with('#') {
                        return self.resolve_package_import(directory, &target, depth + 1);
                    }
                    if let Some(relative) = target.strip_prefix("./") {
                        let path = directory.join(relative).clean();
                        if !path.starts_with(directory) {
                            return Err(Error::from_boxed(
                                eyre::eyre!(
                                    "ERR_INVALID_PACKAGE_TARGET: target escapes package root"
                                )
                                .into(),
                            ));
                        }
                        let resolved = self.resolve_as_file(&path)?;
                        let resolved = match resolved {
                            Some(path) => Some(path),
                            None => self.resolve_as_directory(&path, false)?,
                        };
                        let resolved = self.wrap(resolved).with_context(|| {
                            format!(
                                "resolving package import target {target:?} at {}",
                                path.display()
                            )
                        })?;
                        return self.rewrite_browser_path(
                            &directory.join(PACKAGE),
                            resolved,
                            request,
                        );
                    }
                    if target.starts_with("node:") || is_core_module(&target) {
                        if self.target_env == TargetEnv::Node {
                            return Ok(PathBuf::from(if target.starts_with("node:") {
                                target
                            } else {
                                format!("node:{target}")
                            }));
                        }
                        if target.starts_with("node:") {
                            return Err(Error::from_boxed(
                                eyre::eyre!("Node builtin {target:?} is unsupported for the browser target; no polyfills are provided").into(),
                            ));
                        }
                    }
                    let resolved = self
                        .resolve_node_modules(directory, &target)
                        .and_then(|resolved| self.wrap(resolved))
                        .with_context(|| {
                            format!(
                                "resolving package import dependency {target:?} from {}",
                                directory.display()
                            )
                        })?;
                    return self.rewrite_browser_path(&directory.join(PACKAGE), resolved, request);
                }
                if directory == self.monorepo_root {
                    break;
                }
            }
            Err(Error::from_boxed(
                eyre::eyre!("ERR_PACKAGE_IMPORT_NOT_DEFINED: {request:?}").into(),
            ))
        })();
        result.with_context(|| {
            format!(
                "resolving package import {request:?} from {}",
                base_dir.display()
            )
        })
    }

    fn resolve_filename(&self, base: &Path, module_specifier: &str) -> Result<PathBuf, Error> {
        debug!(
            "Resolving {} from {:#?} for {:#?}",
            module_specifier, base, self.target_env
        );

        // Apply one exact replacement before filesystem and builtin resolution.
        let target = self
            .alias
            .get(module_specifier)
            .map(String::as_str)
            .unwrap_or(module_specifier);

        if target.starts_with('#') {
            let base_dir = if self.observer.is_file(base) {
                base.parent().unwrap_or(base)
            } else {
                base
            };
            return self.resolve_package_import(base_dir, target, 0);
        }

        // Handle builtin modules for nodejs, including aliased requests.
        if let TargetEnv::Node = self.target_env {
            if target.starts_with("node:") {
                return Ok(PathBuf::from(target));
            }

            if is_core_module(target) {
                return Ok(PathBuf::from(format!("node:{target}")));
            }
        }

        if !self.alias.contains_key(module_specifier)
            && !module_specifier.starts_with('.')
            && !(self.target_env == TargetEnv::Browser && is_core_module(target))
        {
            // Handle absolute path

            let path = Path::new(module_specifier);

            if let Ok(file) = self
                .resolve_as_file(path)
                .or_else(|_| self.resolve_as_directory(path, false))
            {
                if let Ok(file) = self.wrap(file).with_context(|| {
                    format!(
                        "failed to resolve relative module specifier {:#?} from {:#?}",
                        module_specifier, base,
                    )
                }) {
                    return Ok(file);
                }
            }
        }

        let base_dir = if self.observer.is_file(base) {
            let cwd = &Path::new(".");
            base.parent().unwrap_or(cwd)
        } else {
            base
        };

        let target_path = Path::new(target);

        let file_name = {
            if target_path.is_absolute() {
                let path = PathBuf::from(target_path);
                self.resolve_as_file(&path)
                    .or_else(|_| self.resolve_as_directory(&path, true))
                    .and_then(|p| self.wrap(p).with_context(|| {
                        format!(
                            "failed to resolve absolute target path {:#?}",
                            module_specifier,
                        )
                    }))
            } else {
                let mut components = target_path.components();

                if let Some(Component::CurDir | Component::ParentDir) = components.next() {
                    #[cfg(windows)]
                    let path = {
                        use normpath::BasePath;
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
                        .and_then(|p| self.wrap(p).with_context(|| {
                            format!(
                                "failed to resolve non-absolute target path {} (resolved as {})",
                                target,
                                path.display(),
                            )
                        }))
                } else {
                    let result = self
                        .resolve_node_modules(base_dir, target)
                        .and_then(|path: Option<PathBuf>| {
                            let file_path = path.with_context(|| format!("failed find a node_modules path within {} for package {:?} above {}", self.monorepo_root.display(), target, base_dir.display()))?;
                            let current_directory = current_dir()?;
                            let relative_path = diff_paths(&file_path, &current_directory);
                            self.wrap(relative_path).with_context(|| {
                                format!(
                                    "failed to diff node_modules path {} against {}",
                                    file_path.display(),
                                    current_directory.display(),
                                )
                            })
                        });

                    tracing::debug!(
                        "in resolve_filename: resolve_node_module({base_dir}, {target}) -> {result:#?}",
                        base_dir = base_dir.display(),
                        target = target,
                        result = result
                    );
                    result
                }
            }
        }
        .and_then(|path| self.rewrite_browser_path(base, path, module_specifier));

        file_name
    }

    fn rewrite_browser_path(
        &self,
        base: &Path,
        path: PathBuf,
        module_specifier: &str,
    ) -> Result<PathBuf, Error> {
        if let TargetEnv::Browser = self.target_env {
            if let Some((pkg_path, browser_cache)) = self.pkg_json_cache.probe_path_with_inputs(
                self.monorepo_root,
                base,
                Some(&self.observer),
            )? {
                let cache_entry = browser_cache.try_get_cached_or_init(|pkgjson| {
                    PackageJsonRewriteData::create(self, pkg_path, pkgjson)
                })?;
                self.observer.replay(&cache_entry.inputs);

                let as_abspath = join_abspath(self.monorepo_root, &path)?;
                let rewrite = cache_entry.rewrite_browser(&as_abspath)?;
                return self.wrap(Some(rewrite.to_path_buf())).with_context(|| {
                    format!(
                        "failed to rewrite browser path {:#?} for {:#?}",
                        path, module_specifier,
                    )
                });
            }
        }
        Ok(path)
    }
}

impl PathResolver for CachingNodeModulesResolver<'_> {
    fn resolve(&self, base: &Path, module_specifier: &str) -> Result<Resolution, Error> {
        self.resolve_filename(base, module_specifier)
            .map(|path| Resolution { path, slug: None })
    }
}
