mod export_collector_tests;
pub mod graph;
pub mod node_visitor;
pub mod unused_finder_visitor_runner;
mod utils;

use napi::Status;
use napi_derive::napi;
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
    sync::Arc,
};
use swc_core::ecma::loader::{
    resolve::Resolve,
    resolvers::{lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver},
};

use crate::{
    error::NapiLikeError,
    import_resolver::TsconfigPathsJson,
    unused_finder::{
        graph::{Graph, GraphFile},
        unused_finder_visitor_runner::ImportExportInfo,
        utils::{
            process_async_imported_paths, process_executed_paths, process_exports_from,
            process_import_path_ids, process_require_paths, retrieve_files,
        },
    },
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct WalkFileMetaData {
    pub package_name: String,
    pub source_file_path: String,
    pub import_export_info: ImportExportInfo,
}

#[derive(Debug, PartialEq)]
pub enum WalkedFile {
    SourceFile(WalkFileMetaData),
    Nothing,
}

impl Default for WalkedFile {
    fn default() -> Self {
        WalkedFile::Nothing
    }
}

#[derive(Default)]
#[napi(object)]
pub struct FindUnusedItemsConfig {
    pub paths_to_read: Vec<String>,
    pub ts_config_path: String,
    // Files under matching dirs won't be scanned.
    pub skipped_dirs: Vec<String>,
    // List of regex. Named items in the form of `export { foo }` and similar (excluding `default`) matching a regex in this list will not be recorded as imported/exported items.
    // e.g. skipped_items = [".*Props$"] and a file contains a `export type FooProps = ...` statement, FooProps will not be recorded as an exported item.
    // e.g. skipped_items = [".*Props$"] and a file contains a `import { BarProps } from 'bar';` statement, BarProps will not be recorded as an imported item.
    pub skipped_items: Vec<String>,
    // Files such as test files, e.g. ["packages/**/src/tests/**"]
    // items and files imported by matching files will not be marked as used.
    pub files_ignored_imports: Vec<String>,
    pub files_ignored_exports: Vec<String>,
    pub entry_packages: Vec<String>,
}

pub fn find_unused_items(
    config: FindUnusedItemsConfig,
) -> Result<Vec<String>, crate::error::NapiLikeError> {
    let FindUnusedItemsConfig {
        paths_to_read,
        ts_config_path,
        skipped_dirs,
        skipped_items,
        files_ignored_imports: _,
        files_ignored_exports: _,
        entry_packages,
    } = config;
    let entry_packages: HashSet<String> = entry_packages.into_iter().collect();
    let tsconfig = match TsconfigPathsJson::from_path(ts_config_path.clone()) {
        Ok(tsconfig) => tsconfig,
        Err(e) => panic!("Unable to read tsconfig file {}: {}", ts_config_path, e),
    };
    let skipped_dirs = skipped_dirs.iter().map(|s| glob::Pattern::new(s));
    let skipped_dirs: Arc<Vec<glob::Pattern>> = match skipped_dirs.into_iter().collect() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            return Err(crate::error::NapiLikeError {
                status: napi::Status::InvalidArg,
                message: e.msg.to_string(),
            })
        }
    };

    let skipped_items = skipped_items
        .iter()
        .map(|s| regex::Regex::from_str(s.as_str()));
    let skipped_items: Vec<regex::Regex> = match skipped_items.into_iter().collect() {
        Ok(r) => r,
        Err(e) => {
            return Err(crate::error::NapiLikeError {
                status: napi::Status::InvalidArg,
                message: e.to_string(),
            })
        }
    };
    let skipped_items = Arc::new(skipped_items);
    // Walk on all files and retrieve the WalkFileData from them
    let mut flattened_walk_file_data: Vec<WalkFileMetaData> = paths_to_read
        .par_iter()
        .map(|path| {
            let mut walked_files =
                retrieve_files(path, Some(skipped_dirs.to_vec()), skipped_items.clone());
            let walked_files_data: Vec<WalkFileMetaData> = walked_files
                .drain(0..)
                .filter_map(|walked_file| {
                    if let WalkedFile::SourceFile(w) = walked_file {
                        return Some(w);
                    }
                    None
                })
                .collect();
            walked_files_data
        })
        .flatten()
        .collect();

    let total_files = flattened_walk_file_data.len();
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
    let mut files: Vec<GraphFile> = flattened_walk_file_data
        .par_iter_mut()
        .map(|file| {
            process_import_export_info(file, &resolver);
            GraphFile {
                file_path: file.source_file_path.clone(),
                import_export_info: file.import_export_info.clone(),
                is_used: entry_packages.contains(&file.package_name), // mark files from entry_packages as used
                unused_exports: file
                    .import_export_info
                    .exported_ids
                    .iter()
                    .map(|e| e.metadata.export_kind.clone())
                    .collect(),
            }
        })
        .collect();

    let files: HashMap<String, Arc<GraphFile>> = files
        .par_drain(0..)
        .map(|file| (file.file_path.clone(), Arc::new(file)))
        .collect();

    let mut graph = Graph {
        files,
        ..Default::default()
    };

    let entry_files: Vec<String> = flattened_walk_file_data
        .par_iter_mut()
        .filter_map(|file| {
            if entry_packages.contains(&file.package_name) {
                return Some(file.source_file_path.clone());
            }
            None
        })
        .collect();
    let mut frontier = entry_files;
    for i in 0..10_000_000 {
        frontier = graph.bfs_step(frontier);

        if frontier.is_empty() {
            break;
        }
        if i == 10_000_000 {
            return Err(NapiLikeError {
                message: "exceeded max iterations".to_string(),
                status: Status::GenericFailure,
            });
        }
    }

    let unused_files = BTreeMap::from_iter(graph.files.drain().filter(|f| !f.1.is_used));
    let results: Vec<String> = unused_files
        .iter()
        .map(|f| format!("\"{}\",", f.0))
        .collect();
    println!("Total files: {}", &total_files);
    println!("Total used files: {}", (total_files - unused_files.len()));
    println!("Total unused files: {}", unused_files.len());

    Ok(results)
}

fn process_import_export_info(f: &mut WalkFileMetaData, resolver: &dyn Resolve) {
    process_executed_paths(&mut f.import_export_info, &f.source_file_path, resolver);
    process_async_imported_paths(&mut f.import_export_info, &f.source_file_path, resolver);
    process_exports_from(&mut f.import_export_info, &f.source_file_path, resolver);
    process_require_paths(&mut f.import_export_info, &f.source_file_path, resolver);
    process_import_path_ids(&mut f.import_export_info, &f.source_file_path, resolver);
}

#[cfg(test)]
mod test {
    use crate::unused_finder::FindUnusedItemsConfig;

    use super::find_unused_items;

    #[test]
    fn test_error_in_glob() {
        let result = find_unused_items(FindUnusedItemsConfig {
            paths_to_read: vec!["tests/unused_finder".to_string()],
            ts_config_path: "tests/unused_finder/tsconfig.json".to_string(),
            skipped_dirs: vec![".....///invalidpath****".to_string()],
            skipped_items: vec!["[A-Z].*".to_string(), "something".to_string()],
            ..Default::default()
        });
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().message,
            "wildcards are either regular `*` or recursive `**`"
        )
    }

    #[test]
    fn test_error_in_regex() {
        let result = find_unused_items(FindUnusedItemsConfig {
            paths_to_read: vec!["tests/unused_finder".to_string()],
            ts_config_path: "tests/unused_finder/tsconfig.json".to_string(),
            skipped_items: vec!["[A-Z.*".to_string(), "something".to_string()],
            ..Default::default()
        });
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().message,
            "regex parse error:\n    [A-Z.*\n    ^\nerror: unclosed character class"
        )
    }
}
