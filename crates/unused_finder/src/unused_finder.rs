use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use anyhow::Result;
use import_resolver::swc_resolver::MonorepoResolver;
use js_err::JsErr;
use rayon::prelude::*;

use crate::{
    core::{
        create_report_map_from_flattened_files, process_import_export_info, read_allow_list,
        walk_src_files, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport,
    },
    graph::{Graph, GraphFile},
    unused_finder_visitor_runner::get_import_export_paths_map,
    walked_file::UnusedFinderSourceFile,
};

#[derive(Debug)]
pub struct UnusedFinder {
    pub report: UnusedFinderReport,
    pub logs: Vec<String>,
    config: FindUnusedItemsConfig,
    entry_packages: HashSet<String>,
    file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>>,
    skipped_items: Arc<Vec<regex::Regex>>,
    skipped_dirs: Arc<Vec<glob::Pattern>>,
    entry_files: Vec<String>,
    graph: Graph,
    resolver: MonorepoResolver,
}

impl UnusedFinder {
    pub fn new(config: FindUnusedItemsConfig) -> anyhow::Result<Self, JsErr> {
        let FindUnusedItemsConfig {
            report_exported_items: _,
            paths_to_read,
            ts_config_path,
            skipped_dirs,
            skipped_items,
            files_ignored_imports: _,
            files_ignored_exports: _,
            entry_packages,
        } = config.clone();
        let root_dir: PathBuf = {
            // scope here to contain the mutability
            let mut x = PathBuf::from(ts_config_path);
            x.pop();
            x
        };
        let resolver: MonorepoResolver = MonorepoResolver::new_default_resolver(root_dir);
        let entry_packages: HashSet<String> = entry_packages.into_iter().collect();

        let skipped_dirs = skipped_dirs.iter().map(|s| glob::Pattern::new(s));
        let skipped_dirs: Arc<Vec<glob::Pattern>> = match skipped_dirs.into_iter().collect() {
            Ok(v) => Arc::new(v),
            Err(e) => {
                // return None;
                return Err(JsErr::invalid_arg(e));
            }
        };

        let skipped_items = skipped_items
            .iter()
            .map(|s| regex::Regex::from_str(s.as_str()));
        let skipped_items: Vec<regex::Regex> = match skipped_items.into_iter().collect() {
            Ok(r) => r,
            Err(e) => {
                return Err(JsErr::invalid_arg(e));
            }
        };
        let skipped_items = Arc::new(skipped_items);
        let mut flattened_walk_file_data =
            walk_src_files(&paths_to_read, &skipped_dirs, &skipped_items);

        let mut files: Vec<GraphFile> = flattened_walk_file_data
            .par_iter_mut()
            .map(|file| -> Result<GraphFile> {
                process_import_export_info(
                    &mut file.import_export_info,
                    &file.source_file_path,
                    &resolver,
                )?;
                Ok(GraphFile::new(
                    file.source_file_path.clone(),
                    file.import_export_info
                        .exported_ids
                        .iter()
                        .map(|e| e.metadata.export_kind.clone())
                        .collect(),
                    file.import_export_info.clone(),
                    entry_packages.contains(&file.package_name), // mark files from entry_packages as used
                ))
            })
            .collect::<Result<Vec<GraphFile>>>()
            .map_err(JsErr::generic_failure)?;

        let files: HashMap<String, Arc<GraphFile>> = files
            .par_drain(0..)
            .map(|file| (file.file_path.clone(), Arc::new(file)))
            .collect();

        let graph = Graph { files };

        let file_path_exported_items_map =
            create_report_map_from_flattened_files(&flattened_walk_file_data);

        Ok(Self {
            config,
            entry_packages,
            skipped_dirs,
            skipped_items,
            resolver: resolver,
            graph,
            file_path_exported_items_map,
            // These fields are empty because they are populated by
            // the mutation methods below (refresh_file_list, walk_file_graph, etc)
            entry_files: Default::default(),
            report: Default::default(),
            logs: Default::default(),
        })
    }

    // Read and parse all files from disk have a fresh in-memory representation of self.entry_files and self.graph information

    pub fn refresh_file_list(&mut self) {
        // Get a vector with all SourceFile
        let mut flattened_walk_file_data = walk_src_files(
            &self.config.paths_to_read,
            &self.skipped_dirs,
            &self.skipped_items,
        );
        let logs = &mut self.logs;
        // Proccess all information with swc resolver to resolve symbols within tsconfig.paths.json
        let errs = flattened_walk_file_data
            .par_iter_mut()
            .map(|file| {
                process_import_export_info(
                    &mut file.import_export_info,
                    &file.source_file_path,
                    &self.resolver,
                )
            })
            .filter_map(|e| {
                if let Err(e) = e {
                    return Some(e);
                }
                None
            })
            .collect::<Vec<_>>();
        for err in errs {
            logs.push(format!("Error processing import/export info: {:?}", err));
        }

        // Refresh graph representation with latest changes from disk
        self.refresh_graph(&flattened_walk_file_data);

        // Create a record of all files listed as entry points, this will serve during `find_unused_items` as the first `frontier` iteration
        self.entry_files = flattened_walk_file_data
            .par_iter()
            .filter_map(|file| {
                if self.entry_packages.contains(&file.package_name) {
                    return Some(file.source_file_path.clone());
                }
                None
            })
            .collect();

        // Create a report map with all the files
        self.file_path_exported_items_map =
            create_report_map_from_flattened_files(&flattened_walk_file_data);
    }

    // Given a Vec<SourceFile> refreshes the in-memory representation of imports/exports of source file graph
    fn refresh_graph(&mut self, flattened_walk_file_data: &Vec<UnusedFinderSourceFile>) {
        let files: HashMap<String, Arc<GraphFile>> = flattened_walk_file_data
            .par_iter()
            .map(|file| {
                (
                    file.source_file_path.to_string(),
                    Arc::new(GraphFile::new(
                        file.source_file_path.clone(),
                        file.import_export_info
                            .exported_ids
                            .iter()
                            .map(|e| e.metadata.export_kind.clone())
                            .collect(),
                        file.import_export_info.clone(),
                        self.entry_packages.clone().contains(&file.package_name), // mark files from entry_packages as used
                    )),
                )
            })
            .collect();

        self.graph = Graph { files };
    }

    pub fn find_all_unused_items(&mut self) -> Result<UnusedFinderReport, JsErr> {
        self.refresh_file_list();
        // Clone file_path_exported_items_map to avoid borrow checker issues with mutable/immutable references of `self`.
        let file_path_exported_items_map = self.file_path_exported_items_map.clone();

        if let Some(value) = self.walk_file_graph() {
            return value;
        }

        let allow_list: Vec<glob::Pattern> = read_allow_list().map_err(JsErr::generic_failure)?;

        let reported_unused_files = self.get_unused_files(allow_list);
        let unused_files_items = self.get_unused_items_file(file_path_exported_items_map);

        let ok = UnusedFinderReport {
            unused_files: reported_unused_files
                .iter()
                .map(|(p, _)| p.to_string())
                .collect(),
            unused_files_items,
        };
        Ok(ok)
    }

    pub fn find_file_unused_items() {}

    pub fn find_unused_items(
        &mut self,
        files_to_check: Vec<String>,
    ) -> Result<UnusedFinderReport, JsErr> {
        self.logs = vec![];
        self.logs.push(format!("{:?}", &files_to_check));
        if files_to_check.is_empty() {
            self.logs.push("Using local representation".to_string());
        } else if !files_to_check
            .iter()
            .any(|file| !self.file_path_exported_items_map.contains_key(file))
        {
            self.logs.push("Refreshing all files".to_string());
            self.refresh_file_list();
        } else {
            self.logs.push("Refreshing only some files!".to_string());
            self.refresh_files_to_check(&files_to_check);
        }

        // Clone file_path_exported_items_map to avoid borrow checker issues with mutable/immutable references of `self`.
        let file_path_exported_items_map = self.file_path_exported_items_map.clone();

        if let Some(value) = self.walk_file_graph() {
            return value;
        }

        let allow_list: Vec<glob::Pattern> = read_allow_list().map_err(JsErr::generic_failure)?;

        let reported_unused_files = self.get_unused_files(allow_list);
        let unused_files_items = self.get_unused_items_file(file_path_exported_items_map);

        let mut ok = UnusedFinderReport {
            unused_files: reported_unused_files
                .iter()
                .map(|(p, _)| p.to_string())
                .collect(),
            unused_files_items,
        };
        let files: HashSet<String> = HashSet::from_iter(files_to_check);
        ok.unused_files_items.retain(|key, _| files.contains(key));
        return Ok(ok);
    }

    fn get_unused_items_file(
        &self,
        mut file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>>,
    ) -> HashMap<String, Vec<ExportedItemReport>> {
        let unused_files_items: HashMap<String, Vec<ExportedItemReport>> = self
            .graph
            .files
            .iter()
            .filter_map(|(file_path, info)| {
                if info.is_used {
                    match file_path_exported_items_map.remove(file_path) {
                        Some(mut exported_items) => {
                            let unused_exports = &info.unused_exports;
                            if unused_exports.is_empty() {
                                return None;
                            }
                            let unused_exports = exported_items
                                .drain(0..)
                                .filter(|exported| {
                                    unused_exports
                                        .iter()
                                        .any(|unused| unused.to_string() == exported.id.to_string())
                                })
                                .collect();
                            return Some((file_path.to_string(), unused_exports));
                        }
                        None => return None,
                    }
                }
                None
            })
            .collect();
        unused_files_items
    }

    fn get_unused_files(
        &self,
        allow_list: Vec<glob::Pattern>,
    ) -> BTreeMap<&String, &Arc<GraphFile>> {
        let reported_unused_files =
            BTreeMap::from_iter(self.graph.files.iter().filter(|(file_name, graph_file)| {
                !graph_file.is_used && !allow_list.iter().any(|p| p.matches(file_name))
            }));
        reported_unused_files
    }

    fn walk_file_graph(&mut self) -> Option<Result<UnusedFinderReport, JsErr>> {
        let mut frontier = self.entry_files.clone();
        for _ in 0..10_000_000 {
            frontier = self.graph.bfs_step(frontier);
            if frontier.is_empty() {
                break;
            }
        }
        if !frontier.is_empty() {
            return Some(Err(JsErr::generic_failure(anyhow!(
                "exceeded max iterations"
            ))));
        }
        None
    }

    // Reads files from disk and updates information within `self.graph` for specified paths in `files_to_check`
    fn refresh_files_to_check(&mut self, files_to_check: &Vec<String>) {
        let results = files_to_check
            .iter()
            .map(|f| -> Result<Option<String>> {
                // Read/parse file from disk
                let visitor_result =
                    get_import_export_paths_map(f.to_string(), self.skipped_items.clone());
                let ok = match visitor_result {
                    Ok(ok) => ok,
                    Err(e) => return Err(anyhow!("Error reading file {}: {:?}", f, e)),
                };

                let current_graph_file = match self.graph.files.get_mut(f) {
                    Some(current_graph_file) => current_graph_file,
                    None => return Ok(None),
                };

                // Check file exists in graph
                let current_graph_file = Arc::get_mut(current_graph_file).unwrap();
                current_graph_file.import_export_info = ok; // Update import_export_info within self.graph
                return match process_import_export_info(
                    // Process import/export info to use resolver.
                    &mut current_graph_file.import_export_info,
                    &f,
                    &self.resolver,
                ) {
                    Ok(_) => Ok(Some(current_graph_file.file_path.clone())),
                    Err(e) => Err(e.context(format!("Error processing file: {:?}", f))),
                };
            })
            .collect::<Vec<_>>();

        // this has to be done in this scope after the above iterator is fully processed,
        // to avoid borrowing issues with `self.logs`
        for res in results {
            match res {
                Ok(Some(file_path)) => {
                    self.logs
                        .push(format!("Refreshed file path: {:?}", file_path));
                }
                Ok(None) => {}
                Err(err) => {
                    self.logs.push(format!("Error refreshing file: {:?}", err));
                }
            }
        }
    }
}
