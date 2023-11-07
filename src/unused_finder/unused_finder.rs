use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
    sync::Arc,
};

use rayon::prelude::*;
use swc_core::ecma::loader::resolvers::{
    lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver,
};

use crate::import_resolver::TsconfigPathsJson;

use super::{
    create_caching_resolver, create_flattened_walked_files, create_report_map_from_flattened_files,
    graph::{Graph, GraphFile},
    process_import_export_info, read_allow_list,
    unused_finder_visitor_runner::get_import_export_paths_map,
    ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport, WalkFileMetaData,
};

#[derive(Debug, Default)]
pub struct UnusedFinder {
    pub report: UnusedFinderReport,
    pub logs: Vec<String>,
    entry_packages: HashSet<String>,
    // Hashmap containing metadata necessary to locate unused exported items within files in vscode diagnostics
    file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>>,
    skipped_items: Arc<Vec<regex::Regex>>,
    skipped_dirs: Arc<Vec<glob::Pattern>>,
    src_files: Vec<WalkFileMetaData>,
    test_files: Vec<WalkFileMetaData>,
    entry_files: Vec<String>,
    paths_to_read: Vec<String>,
    resolver: Option<CachingResolver<TsConfigResolver<NodeModulesResolver>>>,
}


impl UnusedFinder {
    pub fn new(config: FindUnusedItemsConfig) -> napi::Result<Self> {
        let FindUnusedItemsConfig {
            paths_to_read,
            ts_config_path,
            skipped_dirs,
            skipped_items,
            files_ignored_imports: _,
            files_ignored_exports: _,
            entry_packages,
        } = config.clone();
        let tsconfig: TsconfigPathsJson = match TsconfigPathsJson::from_path(ts_config_path.clone())
        {
            Ok(tsconfig) => tsconfig,
            Err(e) => {
                return Err(napi::Error::new(
                    napi::Status::InvalidArg,
                    format!("Unable to read tsconfig file {}: {}", ts_config_path, e),
                ));
            }
        };
        let resolver: CachingResolver<TsConfigResolver<NodeModulesResolver>> =
            create_caching_resolver(&tsconfig);
        let entry_packages: HashSet<String> = entry_packages.into_iter().collect();

        let skipped_dirs = skipped_dirs.iter().map(|s| glob::Pattern::new(s));
        let skipped_dirs: Arc<Vec<glob::Pattern>> = match skipped_dirs.into_iter().collect() {
            Ok(v) => Arc::new(v),
            Err(e) => {
                // return None;
                return Err(napi::Error::new(
                    napi::Status::InvalidArg,
                    e.msg.to_string(),
                ));
            }
        };

        let skipped_items = skipped_items
            .iter()
            .map(|s| regex::Regex::from_str(s.as_str()));
        let skipped_items: Vec<regex::Regex> = match skipped_items.into_iter().collect() {
            Ok(r) => r,
            Err(e) => {
                return Err(napi::Error::new(napi::Status::InvalidArg, e.to_string()));
            }
        };
        let skipped_items = Arc::new(skipped_items);
        let all_files =
            retrieve_and_process_files(&paths_to_read, &skipped_dirs, &skipped_items, &resolver);

        let mut source_files: Vec<WalkFileMetaData> = Vec::with_capacity(all_files.len());

        let mut test_files: Vec<WalkFileMetaData> = vec![];

        for file in all_files {
            if file.is_test_file {
                test_files.push(file);
            } else {
                source_files.push(file);
            }
        }

        let file_path_exported_items_map = create_report_map_from_flattened_files(&source_files);

        Ok(Self {
            entry_packages,
            skipped_dirs,
            skipped_items,
            resolver: Some(resolver),
            file_path_exported_items_map,
            src_files: source_files,
            test_files,
            ..Default::default()
        })
    }

    // Read and parse all files from disk have a fresh in-memory representation of self.entry_files and self.graph information
    
    pub fn refresh_all_files(&mut self) {
        // Get a vector with all WalkFileMetaData
        let all_files = retrieve_and_process_files(
            &self.paths_to_read,
            &self.skipped_dirs,
            &self.skipped_items,
            self.resolver
                .as_ref()
                .expect("Unable to find node modules resolver"),
        );
        let mut source_files: Vec<WalkFileMetaData> = Vec::with_capacity(all_files.len());

        let mut test_files: Vec<WalkFileMetaData> = vec![];

        for file in all_files {
            if file.is_test_file {
                test_files.push(file);
            } else {
                source_files.push(file);
            }
        }

        // Create a report map with all the files
        self.file_path_exported_items_map = create_report_map_from_flattened_files(&source_files);

        // Create a record of all files listed as entry points, this will serve during `find_unused_items` as the first `frontier` iteration
        self.entry_files = source_files
            .par_iter()
            .filter_map(|file| {
                if self.entry_packages.contains(&file.package_name) {
                    return Some(file.source_file_path.clone());
                }
                None
            })
            .collect();

        self.src_files = source_files;

        self.test_files = test_files;
    }

    
    pub fn find_unused_items(
        &mut self,
        files_to_check: Vec<String>,
    ) -> napi::Result<UnusedFinderReport> {
        self.logs = vec![];
        self.logs.push(format!("{:?}", &files_to_check));
        // Create graph containing only src_files
        let mut graph = create_graph(&self.src_files, &self.entry_packages);
        if files_to_check
            .iter()
            .any(|file| !self.file_path_exported_items_map.contains_key(file))
        {
            self.logs.push("Refreshing all files".to_string());
            self.refresh_all_files();
        } else {
            self.logs.push("Refreshing only some files!".to_string());
            self.refresh_files_to_check(&files_to_check, &mut graph);
        }
        // Clone file_path_exported_items_map to avoid borrow checker issues with mutable/immutable references of `self`.
        let file_path_exported_items_map = self.file_path_exported_items_map.clone();
        // let entry_packages = self.entry_packages;
        // Create binding to entry_packages to avoid borrow checker complain about borrowing `self`
        let entry_packages = &self.entry_packages;
        let entry_pkgs_files: Vec<String> = self
        .src_files
        .par_iter_mut()
        .filter_map(|file| {
            if entry_packages.contains(&file.package_name) {
                return Some(file.source_file_path.clone());
            }
            None
        })
        .collect();
        let mut frontier = entry_pkgs_files.clone();
        for _ in 0..10_000_000 {
            frontier = graph.bfs_step(frontier);
        }
        if !frontier.is_empty() {
            return Err(napi::Error::new(
                napi::Status::GenericFailure,
                "exceeded max iterations".to_string(),
            ));
        }
        // Read `.unusedignore` file and retrieve list/patterns to ignore
        let allow_list: Vec<glob::Pattern> = read_allow_list();

        // Create map where key: list of used files, value: Vec containing items exported but never imported
        let unused_items = create_used_files_unused_items_map(&graph, file_path_exported_items_map);

        let reported_unused_files = BTreeMap::from_iter(graph.files.iter().filter(|f| {
            !f.1.is_test_file && !f.1.is_used && !allow_list.iter().any(|p| p.matches(f.0))
        }));


        // Create Vec containing the string paths of source_files + test_files
        let all_files: Vec<_> = self
            .test_files
            .clone()
            .par_drain(0..)
            .chain(self.src_files.clone().par_drain(0..))
            .collect();
        let entry_files = self
            .test_files
            .par_iter()
            .map(|f| f.source_file_path.to_string());
        let file_path_exported_items_map = create_report_map_from_flattened_files(&all_files);
        let mut graph = create_graph(&all_files, &self.entry_packages);

        let mut frontier: Vec<String> = entry_files.collect();
        for _ in 0..10_000_000 {
            frontier = graph.bfs_step(frontier);
        }
        if !frontier.is_empty() {
            return Err(napi::Error::new(
                napi::Status::GenericFailure,
                "exceeded max iterations".to_string(),
            ));
        }

        let test_unused_items: HashMap<String, Vec<ExportedItemReport>> =
            create_used_files_unused_items_map(&graph, file_path_exported_items_map);

        // Get the files list that were only imported by test files
        let test_only_used_files: Vec<String> = test_unused_items
            .iter()
            .filter_map(|(file, _)| {
                if unused_items.contains_key(file) {
                    return Some(file.to_string());
                }
                None
            })
            .collect();

        // Create a map where:
        // Key: file reachable from entry points
        // Value: List of items that were only imported by test files
        let mut test_only_used_items: HashMap<String, Vec<ExportedItemReport>> = test_unused_items
            .iter()
            .filter_map(|(file, items)| {
                if let Some(items_prod) = unused_items.get(file) {
                    // if something is unused in prod but used in tests
                    return Some((
                        file.to_string(),
                        items_prod
                            .iter()
                            .filter(|item| !items.contains(&item))
                            .map(|i| i.clone())
                            .collect::<Vec<_>>(),
                    ));
                }
                None
            })
            .collect();
        test_only_used_items.retain(|_, v| !v.is_empty());

        let mut ok = UnusedFinderReport {
            unused_files: reported_unused_files
                .iter()
                .map(|(p, _)| p.to_string())
                .collect(),
            unused_files_items: test_unused_items,
            test_only_used_files,
            test_only_used_items,
        };
        let files: HashSet<String> = HashSet::from_iter(files_to_check);
        ok.unused_files_items.retain(|key, _| files.contains(key));
        return Ok(ok);
    }

    // Reads files from disk and updates information within `self.graph` for specified paths in `files_to_check`
    fn refresh_files_to_check(&mut self, files_to_check: &Vec<String>, graph: &mut Graph) {
        files_to_check.iter().for_each(|f| {
            // Read/parse file from disk
            let visitor_result =
                get_import_export_paths_map(f.to_string(), self.skipped_items.clone());
            if let Ok(ok) = visitor_result {
                match graph.files.get_mut(f) {
                    // Check file exists in graph
                    Some(current_graph_file) => {
                        let current_graph_file = Arc::get_mut(current_graph_file).unwrap();
                        current_graph_file.import_export_info = ok; // Update import_export_info within self.graph
                        self.logs.push(format!("refreshing {}", f.to_string()));
                        process_import_export_info(
                            // Process import/export info to use resolver.
                            &mut current_graph_file.import_export_info,
                            &f,
                            &self.resolver.as_ref().unwrap(),
                        );
                    }
                    None => todo!(),
                }
            }
        });
    }
}

fn create_used_files_unused_items_map(
    graph: &Graph,
    mut file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>>,
) -> HashMap<String, Vec<ExportedItemReport>> {
    let unused_items: HashMap<String, Vec<ExportedItemReport>> = graph
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
    unused_items
}

fn create_graph(source_files: &Vec<WalkFileMetaData>, entry_packages: &HashSet<String>) -> Graph {
    let mut files = create_graph_files(source_files, entry_packages);

    let files: HashMap<String, Arc<GraphFile>> = files
        .par_drain(0..)
        .map(|file| (file.file_path.clone(), Arc::new(file)))
        .collect();

    let graph = Graph { files };
    graph
}

fn retrieve_and_process_files(
    paths_to_read: &Vec<String>,
    skipped_dirs: &Arc<Vec<glob::Pattern>>,
    skipped_items: &Arc<Vec<regex::Regex>>,
    resolver: &CachingResolver<TsConfigResolver<NodeModulesResolver>>,
) -> Vec<WalkFileMetaData> {
    let mut all_files = create_flattened_walked_files(&paths_to_read, skipped_dirs, skipped_items);

    all_files.par_iter_mut().for_each(|file| {
        process_import_export_info(
            &mut file.import_export_info,
            &file.source_file_path,
            resolver,
        );
    });
    all_files
}

fn create_graph_files(
    source_files: &Vec<WalkFileMetaData>,
    entry_packages: &HashSet<String>,
) -> Vec<GraphFile> {
    let files: Vec<GraphFile> = source_files
        .par_iter()
        .map(|file| {
            GraphFile::new(
                file.source_file_path.clone(),
                file.import_export_info
                    .exported_ids
                    .iter()
                    .map(|e| e.metadata.export_kind.clone())
                    .collect(),
                file.import_export_info.clone(),
                entry_packages.contains(&file.package_name) || file.is_test_file, // mark files from entry_packages as used
                file.is_test_file,
            )
        })
        .collect();
    files
}
