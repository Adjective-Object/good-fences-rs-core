use core::option::Option::None;
use std::path::{Path, PathBuf};

use crate::{
    cfg::{UnusedFinderConfig, UnusedFinderJSONConfig},
    graph::Graph,
    ignore_file::IgnoreFile,
    parse::{get_file_import_export_info, ExportedSymbol},
    report::UnusedFinderReport,
    tag::UsedTag,
    walk::{walk_src_files, RepoPackages, WalkedFiles},
    walked_file::ResolvedSourceFile,
};
use ahashmap::AHashMap;
use anyhow::{Context, Result};
use import_resolver::swc_resolver::{
    combined_resolver::CombinedResolverCaches,
    internal_resolver::InternalOnlyResolver,
    node_resolver::{NodeModulesResolverOptions, DEFAULT_EXPORT_CODITIONS, DEFAULT_EXTENSIONS},
    MonorepoResolver,
};
use js_err::JsErr;
use logger::{debug_logf, Logger};
use rayon::{iter::Either, prelude::*};
use swc_ecma_loader::{resolve::Resolve, TargetEnv};

#[derive(Debug)]
enum DirtyFiles {
    All,
    Some(Vec<PathBuf>),
}

// Holds an in-memory representation of the file tree.
// That representation can be used used to find unused files and exports
// within a project
//
// To use, create a new UnusedFinder, then call `find_unused` to get the accounting
// of unused files and exports.
#[derive(Debug)]
pub struct UnusedFinder {
    config: UnusedFinderConfig,

    // absolute paths of files which have been explicitly marked dirty
    // since the last time we checked for unused files
    dirty_files: DirtyFiles,
    last_walk_result: SourceFiles,
}

/// In-memory representation of the file tree, where imports have been resolved
/// to file-paths.
#[derive(Debug)]
struct SourceFiles {
    /// The packages discovered during the walk, with some additional metadata
    /// in order to allow looking up by either path or package name.
    packages: RepoPackages,
    /// Map of source file paths to the resolved source files
    source_files: AHashMap<PathBuf, ResolvedSourceFile>,
    /// List of "ignore" files discovered during the walk, which denote files that
    /// should be ignored entirely when checking for unused symbols. Those files
    /// are recursively ignored.
    ignore_files: Vec<IgnoreFile>,
}

impl SourceFiles {
    fn try_resolve(
        walk_result: WalkedFiles,
        resolver: impl Resolve + Sync,
    ) -> Result<SourceFiles, anyhow::Error> {
        // Map of source file path to source file data
        let (source_files, errors): (AHashMap<PathBuf, ResolvedSourceFile>, Vec<anyhow::Error>) =
            walk_result
                .source_files
                .into_par_iter()
                .map(|walked_file| -> Result<(PathBuf, ResolvedSourceFile)> {
                    Ok((
                        walked_file.source_file_path.clone(),
                        ResolvedSourceFile {
                            import_export_info: walked_file
                                .import_export_info
                                .try_resolve(&walked_file.source_file_path, &resolver)
                                .into_anyhow()
                                .with_context(|| {
                                    format!(
                                        "trying to resolve imports for file {}",
                                        walked_file.source_file_path.display()
                                    )
                                })?,
                            owning_package: walked_file.owning_package,
                            source_file_path: walked_file.source_file_path,
                        },
                    ))
                })
                .partition_map::<AHashMap<PathBuf, ResolvedSourceFile>, _, _, _, _>(|r| match r {
                    Ok(x) => Either::Left(x),
                    Err(e) => Either::Right(e),
                });

        if !errors.is_empty() {
            if errors.len() == 1 {
                return Err(errors.into_iter().next().unwrap());
            } else {
                return Err(anyhow!(
                    "Multiple errors occurred during resolution:\n{}",
                    errors
                        .into_iter()
                        .map(|x| format!("{:#}", x))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }

        Ok(SourceFiles {
            source_files,
            packages: walk_result.packages,
            ignore_files: walk_result.ignore_files,
        })
    }
}

fn resolver_for_packages(root_dir: PathBuf, packages: &RepoPackages) -> impl Resolve {
    let mut caches = CombinedResolverCaches::new();
    // pre-populate the packagejson cache with the loaded package json files
    let pkg_caches = caches.package_json_cache();
    for package in packages.packages.iter() {
        pkg_caches.prepopulate(&package.package_path, package.package_json.clone());
    }

    // TODO: rewrite the monorepo resolver to use an abstract filesystem that supports caching I/O
    // then, use that to prepopulate the locations of files on disk. That will short-circuit the
    // resolver going to disk.

    // create a new monorepo resolver
    let mut resolver_options = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
    // include assets during resolution
    let ext_iter = DEFAULT_EXTENSIONS.iter().chain([".svg", ".bmp"].iter());
    // also include d.* extensions during resolution (e.g. "d.ts")
    resolver_options.extensions = ext_iter
        .clone()
        .map(|x| x.to_string())
        .chain(ext_iter.map(|x| format!("{}{}", "d.", x)))
        .collect::<Vec<String>>();
    // also include "source" import conditions during resolution
    resolver_options.export_conditions = ["source"]
        .iter()
        .chain(DEFAULT_EXPORT_CODITIONS)
        .map(|x| x.to_string())
        .collect::<Vec<String>>();

    let monorepo_resolver = MonorepoResolver::new_for_caches(root_dir, caches, resolver_options);
    // create a new resolver that uses the source files to resolve imports
    let walked_files_resolver =
        InternalOnlyResolver::new_with_package_names(monorepo_resolver, packages.iter_names());

    walked_files_resolver
}

impl UnusedFinder {
    pub fn new_from_json_config(
        logger: impl Logger + Sync,
        json_config: UnusedFinderJSONConfig,
    ) -> Result<Self, JsErr> {
        let config = UnusedFinderConfig::try_from(json_config).map_err(JsErr::invalid_arg)?;
        Self::new_from_cfg(logger, config).map_err(JsErr::generic_failure)
    }

    pub fn new_from_cfg(
        logger: impl Logger + Sync,
        config: UnusedFinderConfig,
    ) -> Result<Self, JsErr> {
        if config.repo_root.is_empty() {
            return Err(JsErr::invalid_arg(anyhow!(
                "repoRoot must be set in config"
            )));
        }

        // perform initial walk on initialization to get an internal representation of source files
        let resolved_walked_files = Self::walk_and_resolve_all(logger, &config)?;

        Ok(Self {
            config,
            dirty_files: DirtyFiles::Some(vec![]),
            last_walk_result: resolved_walked_files,
        })
    }

    // Read and parse all files from disk have a fresh in-memory representation of the file tree
    pub fn mark_dirty<I, Item>(&mut self, file_paths: I)
    where
        I: IntoIterator<Item = Item, IntoIter: Clone>,
        Item: AsRef<Path>,
    {
        let iterator = file_paths.into_iter();
        if iterator.clone().any(|path| {
            // If any of the files are not in the last_walk_result, mark all files as dirty
            !self.last_walk_result.source_files.contains_key(path.as_ref())
            // If any of the files are packagejson files, mark all files as dirty
            || self.last_walk_result.packages.contains_path(path.as_ref())
        }) {
            self.dirty_files = DirtyFiles::All;
            return;
        }

        // Add to the current list of dirty files, if it's not already marked as all dirty
        if let DirtyFiles::Some(ref mut files) = self.dirty_files {
            // Add the new files to the list of dirty files
            for file_path in iterator {
                files.push(file_path.as_ref().to_path_buf());
            }
        }
    }

    // Marks all files as dirty, so that the next call to `find_unused` will refresh the entire file tree
    pub fn mark_all_dirty(&mut self) {
        self.dirty_files = DirtyFiles::All;
    }

    // Helper method used before taking a snapshot of the file tree for graph computation.
    // performs either a partial or full refresh of the file tree, depending on the value of `files_to_check`
    fn update_dirty_files(&mut self, logger: impl Logger + Sync) -> Result<(), JsErr> {
        match &self.dirty_files {
            DirtyFiles::All => {
                logger.log("Refreshing all files");
                // perform initial walk on initialization to get an internal representation of source files
                self.last_walk_result = Self::walk_and_resolve_all(logger, &self.config)?;
            }
            DirtyFiles::Some(files) => {
                if files.is_empty() {
                    return Ok(());
                }
                logger.log("Refreshing only the files that have been marked dirty");
                let scanned_files = files
                    .par_iter()
                    .map(|file_path| {
                        self.update_single_file(file_path, &logger)
                            .map_err(JsErr::generic_failure)
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                for (file_path, scanned_file) in files.iter().zip(scanned_files) {
                    self.last_walk_result
                        .source_files
                        // TODO: use entry_ref to update in-place if we migrate to hashbrown,
                        // instead of cloning the key here
                        .insert(file_path.clone(), scanned_file);
                }
            }
        }

        // clear the list of dirty files
        self.dirty_files = DirtyFiles::Some(vec![]);
        Ok(())
    }

    fn update_single_file(
        &self,
        file_path: &Path,
        logger: impl Logger,
    ) -> Result<ResolvedSourceFile, JsErr> {
        let owning_package = match self.last_walk_result.source_files.get(file_path) {
            Some(existing) => existing.owning_package.clone(),
            None => {
                return Err(JsErr::generic_failure(anyhow!(
                    "Tried to update a file that was not present in the last_walk_result: {}",
                    file_path.display(),
                )));
            }
        };

        let import_export_info = match get_file_import_export_info(file_path) {
            Ok(import_export_info) => import_export_info,
            Err(e) => {
                logger.log(format!(
                    "Error reading file {}: {:?}",
                    file_path.display(),
                    e
                ));
                return Err(JsErr::generic_failure(e));
            }
        };

        let resolver = resolver_for_packages(
            PathBuf::from(self.config.repo_root.clone()),
            &self.last_walk_result.packages,
        );

        let resolved_source_file = ResolvedSourceFile {
            owning_package,
            source_file_path: file_path.to_path_buf(),
            import_export_info: import_export_info
                .try_resolve(file_path, resolver)
                .into_anyhow()
                .map_err(JsErr::generic_failure)?,
        };

        Ok(resolved_source_file)
    }

    /// Walks and parses all source files in the repo, returning a WalkFileResult
    /// with
    fn walk_and_resolve_all(
        logger: impl Logger + Sync,
        config: &UnusedFinderConfig,
    ) -> Result<SourceFiles, JsErr> {
        // Note: this silently ignores any errors that occur during the walk
        let walked_files =
            walk_src_files(&logger, &config.root_paths, &config.repo_root, &config.skip)
                .map_err(JsErr::generic_failure)?;

        let resolver =
            resolver_for_packages(PathBuf::from(&config.repo_root), &walked_files.packages);

        // TODO: gracefully handle errors during resolution
        logger.log(format!(
            "Resolving {} files...",
            walked_files.source_files.len()
        ));
        let resolved =
            SourceFiles::try_resolve(walked_files, resolver).map_err(JsErr::generic_failure)?;
        logger.log("Done resolving files");
        Ok(resolved)
    }

    // Gets a report by performing a graph traversal on the current in-memory state of the repo,
    // from the last time the file tree was scanned.
    pub fn find_unused(&mut self, logger: impl Logger + Sync) -> Result<UnusedFinderResult, JsErr> {
        // Scan the file-system for changed files
        self.update_dirty_files(&logger)?;

        // Create a new graph with all entries marked as "unused".
        let mut graph = Graph::from_source_files(self.last_walk_result.source_files.values());

        // print the entry packages config
        debug_logf!(logger, "Entry packages: {:#?}", self.config.entry_packages);

        // Get the walk roots and perform the graph traversal
        let entrypoints = self.get_entrypoints(&logger);
        logger.log(format!(
            "Starting {} graph traversal with {} entrypoints",
            UsedTag::FROM_ENTRY,
            entrypoints.len()
        ));
        graph
            .traverse_bfs(&logger, entrypoints, vec![], UsedTag::FROM_ENTRY)
            .map_err(JsErr::generic_failure)?;

        let ignored_entrypoints = self.get_ignored_files();
        let ignored_symbols = self.get_ignored_symbols();
        logger.log(format!(
            "Starting {} graph traversal with {} entrypoints and {} symbols",
            UsedTag::FROM_IGNORED,
            ignored_entrypoints.len(),
            Self::count_symbols(&ignored_symbols)
        ));
        graph
            .traverse_bfs(
                &logger,
                ignored_entrypoints,
                ignored_symbols,
                UsedTag::FROM_IGNORED,
            )
            .map_err(JsErr::generic_failure)?;

        let test_entrypoints = self.get_test_files();
        logger.log(format!(
            "Starting {} graph traversal with {} entrypoints",
            UsedTag::FROM_TEST,
            test_entrypoints.len(),
        ));
        graph
            .traverse_bfs(&logger, test_entrypoints, vec![], UsedTag::FROM_TEST)
            .map_err(JsErr::generic_failure)?;

        // mark all typeonly symbols
        if self.config.allow_unused_types {
            for (path, source_file) in self.last_walk_result.source_files.iter() {
                for (_original_path, (symbol, metadata)) in
                    source_file.import_export_info.iter_exported_symbols_meta()
                {
                    // println!("checking symbol: {}:{}", path.display(), symbol);
                    if metadata.is_type_only {
                        // println!("marking typeonly symbol: {}:{}", path.display(), symbol);
                        // By using the file's own path here instead of the iterators' reported path, we are marking
                        // re-exported symbols as used within the file, that re-exports them, NOT within the file they
                        // originate from
                        //
                        // This is because we want to report errors when a typeonly re-export's concrete implementation
                        // is never used.
                        graph.mark_symbol(path, symbol, UsedTag::TYPE_ONLY);
                    }
                }
            }
        }

        Ok(UnusedFinderResult::new(graph))
    }

    fn count_symbols<T, U>(symbols: &[(T, Vec<U>)]) -> usize {
        symbols.iter().map(|(_, symbols)| symbols.len()).sum()
    }

    /// helper to get the list of files that are "entrypoints" to the used
    /// symbol graph (ignored files)
    fn get_entrypoints(&self, logger: impl Logger + Sync) -> Vec<&Path> {
        // get all package exports.
        self.last_walk_result
            .source_files
            .par_iter()
            .filter_map(|(file_path, source_file)| -> Option<&Path> {
                let export = self.is_entry_package_export(&logger, file_path, source_file);
                if export {
                    Some(file_path)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Helper that checks if a file is an export of an "entry package"
    fn is_entry_package_export(
        &self,
        logger: impl Logger,
        file_path: &Path,
        source_file: &ResolvedSourceFile,
    ) -> bool {
        let owning_package_name = match source_file.owning_package {
            Some(ref owning_package) => owning_package,
            None => {
                // TODO: should un-rooted scripts be considered entry points?
                // This might be the case for e.g. tests or other scripts
                logger.log(format!(
                    "Could not find package for source file {}",
                    file_path.display(),
                ));
                return false;
            }
        };

        // get the corresponding package of the source file
        let owning_package = match self
            .last_walk_result
            .packages
            .get_by_name(owning_package_name)
        {
            Some(owning_package) => owning_package,
            None => {
                logger.log(format!(
                    "Could not find owning package {owning_package_name:?} for source file {}",
                    file_path.display(),
                ));
                return false;
            }
        };

        let relative_package_path = owning_package
            .package_path
            .strip_prefix(&self.config.repo_root)
            .expect("absolue paths of packages within the repo should be relative");

        // only "entry packages" may export scripts
        if !self
            .config
            .entry_packages
            .matches(relative_package_path, owning_package_name)
        {
            return false;
        }

        // check if the owning package exports the file. If so, include this file as a package root.
        owning_package
            .is_abspath_exported(file_path)
            .unwrap_or(false)
    }

    fn get_ignored_files(&self) -> Vec<&Path> {
        // TODO: this is n^2, which is bad! Could build a treemap of ignore files?
        self.last_walk_result
            .source_files
            .par_iter()
            .filter_map(|(file_path, _)| -> Option<&Path> {
                if self.is_file_ignored(file_path) {
                    Some(file_path)
                } else {
                    None
                }
            })
            .collect()
    }

    fn get_ignored_symbols(&self) -> Vec<(&Path, Vec<ExportedSymbol>)> {
        self.last_walk_result
            .source_files
            .par_iter()
            .filter_map(|(path_buf, file)| -> Option<(&Path, Vec<ExportedSymbol>)> {
                let ignored_symbols = Self::get_file_ignored_symbols(file);
                if ignored_symbols.is_empty() {
                    None
                } else {
                    Some((path_buf, ignored_symbols))
                }
            })
            .collect()
    }

    fn get_test_files(&self) -> Vec<&Path> {
        self.last_walk_result
            .source_files
            .par_iter()
            .filter_map(|(path, _)| -> Option<&Path> {
                for test_glob in &self.config.test_files {
                    let relative = path.strip_prefix(&self.config.repo_root).unwrap_or(path);
                    if test_glob.matches_path(relative) {
                        return Some(path);
                    }
                }

                None
            })
            .collect()
    }

    /// Helper that checks if a file is ignored by any of the ignore files
    fn is_file_ignored(&self, file_path: &Path) -> bool {
        self.last_walk_result
            .ignore_files
            .iter()
            .any(|ignore_file| ignore_file.is_ignored(file_path))
    }

    /// Helper that checks if a file is ignored by any of the ignore files
    fn get_file_ignored_symbols(file: &ResolvedSourceFile) -> Vec<ExportedSymbol> {
        file.import_export_info
            .exported_ids
            .iter()
            .filter_map(|(symbol, metadata)| {
                if metadata.allow_unused {
                    Some(symbol.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Represents the result of computing something over the graph.
pub struct UnusedFinderResult {
    /// The finished, traversed graph, with unused items marked as used / unused.
    pub graph: Graph,
}

impl UnusedFinderResult {
    pub fn new(graph: Graph) -> Self {
        Self { graph }
    }

    /// Gets a report that can be presented to the JS bridge.
    pub fn get_report(&self) -> UnusedFinderReport {
        UnusedFinderReport::from(self)
    }
}
