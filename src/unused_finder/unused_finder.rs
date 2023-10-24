use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
    sync::Arc,
};

use napi_derive::napi;
use rayon::prelude::*;
use swc_core::{
    common::source_map::Pos,
    ecma::loader::resolvers::{
        lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver,
    },
};

use crate::import_resolver::TsconfigPathsJson;

use super::{
    create_caching_resolver, create_flattened_walked_files, create_report_map_from_flattened_files,
    graph::{Graph, GraphFile},
    process_import_export_info, read_allow_list,
    unused_finder_visitor_runner::get_import_export_paths_map,
    ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport, WalkFileMetaData,
};

#[derive(Debug, Clone, Default)]
#[napi(js_name = "UnusedFinder")]
pub struct UnusedFinder {
    pub report: UnusedFinderReport,
    pub logs: Vec<String>,
    config: FindUnusedItemsConfig,
    entry_packages: HashSet<String>,
    file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>>,
    skipped_items: Arc<Vec<regex::Regex>>,
    skipped_dirs: Arc<Vec<glob::Pattern>>,
    tsconfig: TsconfigPathsJson,
    flattened_walk_file_data: Vec<WalkFileMetaData>,
    graph: Graph,
    // resolver: CachingResolver<TsConfigResolver<NodeModulesResolver>>,
}

#[napi]
impl UnusedFinder {
    #[napi(constructor)]
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
        let resolver: CachingResolver<TsConfigResolver<NodeModulesResolver>> = CachingResolver::new(
            60_000,
            TsConfigResolver::new(
                NodeModulesResolver::default(),
                ".".into(),
                tsconfig
                    .compiler_options
                    .paths
                    .clone()
                    .into_iter()
                    .collect(),
            ),
        );
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
        let mut flattened_walk_file_data =
            create_flattened_walked_files(&paths_to_read, &skipped_dirs, &skipped_items);

        let mut files: Vec<GraphFile> = flattened_walk_file_data
            .par_iter_mut()
            .map(|file| {
                process_import_export_info(file, &resolver);
                GraphFile::new(
                    file.source_file_path.clone(),
                    file.import_export_info
                        .exported_ids
                        .iter()
                        .map(|e| e.metadata.export_kind.clone())
                        .collect(),
                    file.import_export_info.clone(),
                    entry_packages.contains(&file.package_name), // mark files from entry_packages as used
                )
            })
            .collect();

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
            tsconfig,
            skipped_dirs,
            skipped_items,
            flattened_walk_file_data,
            graph,
            file_path_exported_items_map,
            ..Default::default()
        })
    }

    #[napi]
    pub fn refresh_file_list(&mut self) {
        let resolver = create_caching_resolver(&self.tsconfig);
        self.flattened_walk_file_data = create_flattened_walked_files(
            &self.config.paths_to_read,
            &self.skipped_dirs,
            &self.skipped_items,
        );
        self.file_path_exported_items_map = self
            .flattened_walk_file_data
            .clone()
            .par_drain(0..)
            .map(|file| {
                let ids = file
                    .import_export_info
                    .exported_ids
                    .iter()
                    .map(|exported_item| ExportedItemReport {
                        id: exported_item.metadata.export_kind.to_string(),
                        start: exported_item.metadata.span.lo.to_usize() as i32,
                        end: exported_item.metadata.span.hi.to_usize() as i32,
                    })
                    .collect();
                (file.source_file_path.clone(), ids)
            })
            .collect();
        self.flattened_walk_file_data
            .par_iter_mut()
            .for_each(|file| {
                process_import_export_info(file, &resolver);
            });

        self.refresh_graph();
    }

    fn refresh_graph(&mut self) {
        let files: HashMap<String, Arc<GraphFile>> = self
            .flattened_walk_file_data
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

    #[napi]
    pub fn find_unused_items(
        &mut self,
        files_to_check: Vec<String>,
    ) -> napi::Result<UnusedFinderReport> {
        self.logs = vec![];
        self.logs.push(format!("{:?}", &files_to_check));
        if files_to_check
            .iter()
            .any(|file| !self.file_path_exported_items_map.contains_key(file))
        {
            self.logs.push("Refreshing all files".to_string());
            self.refresh_file_list();
            self.refresh_graph();
        } else {
            self.logs.push("Refreshing only some files!".to_string());
            self.refresh_files_to_check(&files_to_check);
        }
        self.file_path_exported_items_map =
            create_report_map_from_flattened_files(&self.flattened_walk_file_data);
        let mut file_path_exported_items_map = self.file_path_exported_items_map.clone();
        let entry_files: Vec<String> = self
            .flattened_walk_file_data
            .par_iter()
            .filter_map(|file| {
                if self.entry_packages.contains(&file.package_name) {
                    return Some(file.source_file_path.clone());
                }
                None
            })
            .collect();
        let mut frontier = entry_files;
        for i in 0..10_000_000 {
            frontier = self.graph.bfs_step(frontier);

            if frontier.is_empty() {
                break;
            }
            if i == 10_000_000 {
                return Err(napi::Error::new(
                    napi::Status::GenericFailure,
                    "exceeded max iterations".to_string(),
                ));
            }
        }

        let allow_list: Vec<glob::Pattern> = read_allow_list();

        let reported_unused_files = BTreeMap::from_iter(
            self.graph
                .files
                .iter()
                .filter(|f| !f.1.is_used && !allow_list.iter().any(|p| p.matches(f.0))),
        );
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

    fn refresh_files_to_check(&mut self, files_to_check: &Vec<String>) {
        files_to_check.iter().for_each(|f| {
            match self
                .flattened_walk_file_data
                .par_iter_mut()
                .find_first(|walk_file_info| &walk_file_info.source_file_path == f)
            {
                Some(current_file) => {
                    let visitor_result =
                        get_import_export_paths_map(f.to_string(), self.skipped_items.clone());
                    if let Ok(ok) = visitor_result {
                        current_file.import_export_info = ok;
                        self.logs.push(format!("refreshing {}", f.to_string()));
                        process_import_export_info(
                            current_file,
                            &create_caching_resolver(&self.tsconfig),
                        );
                    }
                }
                None => todo!(),
            }
        });
    }
}