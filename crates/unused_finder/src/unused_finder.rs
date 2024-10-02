use std::{collections::HashSet, iter::FromIterator, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use import_resolver::swc_resolver::MonorepoResolver;
use js_err::JsErr;
use packagejson::{Browser, PackageJsonExport, StringOrBool};

use crate::{
    core::{
        create_report_map_from_flattened_files, process_import_export_info, read_allow_list,
        walk_src_files, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport,
        WalkFileResult,
    },
    graph::{Graph, GraphFile},
    logger::Logger,
    parse::get_file_import_export_info,
    walked_file::UnusedFinderSourceFile,
};

// the config is designed to be json / javascript serializable,
// so we also have this struct to hold intermediate values that
// can be derived from the config
#[derive(Debug, Clone)]
struct ProcessedConfig {
    entry_packages: HashSet<String>,
    skipped_items: Arc<Vec<regex::Regex>>,
    skipped_dirs: Arc<Vec<glob::Pattern>>,
}

impl TryFrom<&FindUnusedItemsConfig> for ProcessedConfig {
    type Error = JsErr;
    fn try_from(value: &FindUnusedItemsConfig) -> std::result::Result<Self, Self::Error> {
        let skipped_items = value
            .skipped_items
            .iter()
            .map(|s| regex::Regex::from_str(s.as_str()))
            .collect::<Result<Vec<regex::Regex>, _>>()
            .context("while parsing skipped_items as regexp")
            .map_err(JsErr::invalid_arg)?;

        let skipped_dirs = value
            .skipped_dirs
            .iter()
            .map(|s| glob::Pattern::new(s))
            .collect::<Result<Vec<glob::Pattern>, _>>()
            .context("while parsing skipped_dirs as glob patterns")
            .map_err(JsErr::invalid_arg)?;

        Ok(ProcessedConfig {
            entry_packages: value.entry_packages.iter().cloned().collect(),
            skipped_items: Arc::new(skipped_items),
            skipped_dirs: Arc::new(skipped_dirs),
        })
    }
}

#[derive(Debug)]
enum DirtyFiles {
    All,
    Some(Vec<String>),
}

// Holds an in-memory representation of the file tree.
// That representation can be used used to find unused files and exports
// within a project
//
// To use, create a new UnusedFinder, then call `find_unused` to get the accounting
// of unused files and exports.
#[derive(Debug)]
pub struct UnusedFinder {
    config: FindUnusedItemsConfig,
    processed_config: ProcessedConfig,

    // absolute paths of files which have been explicitly marked dirty
    // since the last time we checked for unused files
    dirty_files: DirtyFiles,
    last_walk_result: WalkFileResult,

    // Resolver used to resolve import/export paths during find_unused
    resolver: MonorepoResolver,
}

impl UnusedFinder {
    pub fn new(logger: impl Logger, config: FindUnusedItemsConfig) -> anyhow::Result<Self, JsErr> {
        let p_config: ProcessedConfig = (&config).try_into().map_err(JsErr::generic_failure)?;
        let root_dir: PathBuf = {
            // scope here to contain the mutability
            // HACK: Use the directory of the tsconfig_paths file as the root of the monorepo
            let mut x = PathBuf::from(&config.ts_config_path);
            x.pop();
            x
        };

        let resolver: MonorepoResolver = MonorepoResolver::new_default_resolver(root_dir);

        // perform initial walk on initialization to get an internal representation of source files
        let walked_files = walk_src_files(
            logger,
            &config.root_paths,
            &p_config.skipped_dirs,
            &p_config.skipped_items,
        );

        Ok(Self {
            config,
            processed_config: p_config,
            dirty_files: DirtyFiles::Some(vec![]),
            last_walk_result: walked_files,
            resolver,
        })
    }

    // Read and parse all files from disk have a fresh in-memory representation of the file tree
    pub fn mark_dirty(&mut self, file_paths: Vec<String>) {
        // Get a vector with all SourceFile
        match self.dirty_files {
            DirtyFiles::All => {
                // If we're already marking all files as dirty, don't bother with the rest
                return;
            }
            DirtyFiles::Some(ref mut files) => {
                // Add the new files to the list of dirty files
                for file_path in file_paths {
                    files.push(file_path);
                }
            }
        }
    }

    // Marks all files as dirty, so that the next call to `find_unused` will refresh the entire file tree
    pub fn mark_all_dirty(&mut self) {
        self.dirty_files = DirtyFiles::All;
    }

    // Helper method used before taking a snapshot of the file tree for graph computation.
    // performs either a partial or full refresh of the file tree, depending on the value of `files_to_check`
    fn update_dirty_files(&mut self, logger: impl Logger) -> Result<(), JsErr> {
        match &self.dirty_files {
            DirtyFiles::All => {
                logger.log("Refreshing all files");
                self.last_walk_result = walk_src_files(
                    logger,
                    &self.config.root_paths,
                    &self.processed_config.skipped_dirs,
                    &self.processed_config.skipped_items,
                );
            }
            DirtyFiles::Some(files) => {
                logger.log("Refreshing only the files that have been marked dirty");
                for file_path in files {
                    let visitor_result =
                        // note: this .clone() clones the Arc<> not the underlying Vec<Regex>
                        get_file_import_export_info(file_path.clone(), self.processed_config.skipped_items.clone());
                    let ok = match visitor_result {
                        Ok(import_export_info) => {
                            // Store the updated info in the last_walk_result
                            self.last_walk_result.source_files.insert(
                                file_path.clone(),
                                UnusedFinderSourceFile {
                                    source_file_path: file_path.clone(),
                                    import_export_info,
                                },
                            );
                        }
                        Err(e) => logger.log(format!("Error reading file {}: {:?}", file_path, e)),
                    };
                }
            }
        }
        Ok(())
    }

    // Gets a report by performing a graph traversal on the current in-memory state of the repo,
    // from the last time the file tree was scanned.
    pub fn find_unused(&mut self, logger: impl Logger) -> Result<UnusedItems, JsErr> {
        self.update_dirty_files(logger);

        // Create a new graph with all entries marked as "unused".
        let mut graph = Graph::from_source_files(self.last_walk_result.source_files.values());
        // Start the traversal from the "entry" files.
        let mut frontier = self.get_entry_files();

        for _ in 0..10_000_000 {
            frontier = graph.bfs_step(frontier);
            if frontier.is_empty() {
                return Ok(UnusedItems::from_graph(graph));
            }
        }
        Err(JsErr::generic_failure(anyhow!("exceeded max iterations")))
    }

    fn get_entry_files(&self) -> Vec<String> {
        self.last_walk_result
            .packages
            .iter()
            .filter_map(|(package_name, (packagejson_path, packagejson))| {
                let mut relative_entries = HashSet::<String>::new()
                if self.processed_config.entry_packages.contains(package_name) {
                    // we want to include _all_ entrypoints to the package, regardless of their import condition.
                    // This means that the frontier set will contain many paths that do not currenlty exist on disk,
                    // e.g. because they are not yet compiled.
                    match packagejson.browser {
                        Some(Browser::Obj(browsermap)) => {
                            for (_, value) in browsermap {
                                if let StringOrBool::Str(value) = value {
                                    relative_entries.insert(value);
                                }
                            }
                        }
                        Some(Browser::Str(path)) => {
                            relative_entries.insert(path);
                        }
                        _ => {}
                    }
                    if let Some(pkg_main) = packagejson.main {
                        relative_entries.insert(pkg_main);
                    } 
                    if let Some(pkg_module) = packagejson.module {
                        relative_entries.insert(pkg_module);
                    }

                    if let Some(pkg_exports) = packagejson.exports {
                        match pkg_exports {
                            PackageJsonExport::Single(map) => {
                                for (_, path) in map {
                                    if let Some(path) = path {
                                        relative_entries.insert(path);
                                    }
                                }
                            }
                            PackageJsonExport::Conditional(map) => {
                                for (_, inner_map) in map {
                                    for (_export_condition, path) in inner_map {
                                        if let Some(path) = path {
                                            relative_entries.insert(path);
                                        }
                                    }
                                }
                            }
                        }
                    }



                    None
                } else {
                    None
                }
            })
            .collect()
    }
}

pub struct UnusedItems {
    graph: Graph,
    entrypoints: Vec<String>,
}

impl UnusedItems {
    pub fn from_graph(graph: Graph) -> Self {
        Self { graph }
    }

    pub fn report() -> UnusedFinderReport {}
}
