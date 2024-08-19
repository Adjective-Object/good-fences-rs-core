use anyhow::Result;
use import_resolver::swc_resolver::create_tsconfig_paths_resolver;
use rayon::prelude::*;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
    sync::Arc,
};
use swc_core::{
    common::source_map::Pos,
    ecma::loader::{
        resolve::Resolve,
        resolvers::{lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver},
    },
};

use crate::import_export_info::ImportExportInfo;
use crate::utils::{
    process_async_imported_paths, process_executed_paths, process_exports_from,
    process_import_path_ids, process_require_paths, retrieve_files,
};
use crate::walked_file::UnusedFinderSourceFile;
use crate::{
    graph::{Graph, GraphFile},
    walked_file::WalkedFile,
};
use js_err::JsErr;
use tsconfig_paths::TsconfigPathsJson;

#[cfg_attr(feature = "napi", napi(object))]
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindUnusedItemsConfig {
    // Trace exported symbols that are not imported anywhere in the project
    pub report_exported_items: bool,
    // Paths to read as source files
    pub paths_to_read: Vec<String>,
    // Path to the root tsconfig.paths.json file used to resolve ts imports between projects.
    // Note: this should be removed and replaced with normal node resolution.
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

// Represents a single exported item in a file
#[derive(Debug, Clone)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct ExportedItemReport {
    pub id: String,
    pub start: i32,
    pub end: i32,
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct UnusedFinderReport {
    pub unused_files: Vec<String>,
    pub unused_files_items: HashMap<String, Vec<ExportedItemReport>>,
    // pub flattened_walk_file_data: Vec<WalkedSourceFile>,
}

pub fn create_report_map_from_flattened_files(
    flattened_walk_file_data: &Vec<UnusedFinderSourceFile>,
) -> HashMap<String, Vec<ExportedItemReport>> {
    let file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>> =
        flattened_walk_file_data
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
    file_path_exported_items_map
}

pub fn walk_src_files(
    paths_to_read: &Vec<String>,
    skipped_dirs: &Arc<Vec<glob::Pattern>>,
    skipped_items: &Arc<Vec<regex::Regex>>,
) -> Vec<UnusedFinderSourceFile> {
    let flattened_walk_file_data: Vec<UnusedFinderSourceFile> = paths_to_read
        .par_iter()
        .map(|path| {
            let mut walked_files =
                retrieve_files(path, Some(skipped_dirs.to_vec()), skipped_items.clone());
            let walked_files_data: Vec<UnusedFinderSourceFile> = walked_files
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
    flattened_walk_file_data
}

pub fn find_unused_items(
    config: FindUnusedItemsConfig,
) -> Result<UnusedFinderReport, js_err::JsErr> {
    let FindUnusedItemsConfig {
        report_exported_items,
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
        Err(e) => return Err(JsErr::invalid_arg(e.msg.to_string())),
    };

    let skipped_items: Vec<regex::Regex> = skipped_items
        .iter()
        .map(|s| {
            regex::Regex::from_str(s.as_str())
                // convert regex err to JsErr
                .map_err(JsErr::invalid_arg)
        })
        .collect::<Result<Vec<regex::Regex>, JsErr>>()?;

    let skipped_items = Arc::new(skipped_items);
    // Walk on all files and retrieve the WalkFileData from them
    let mut flattened_walk_file_data: Vec<UnusedFinderSourceFile> =
        walk_src_files(&paths_to_read, &skipped_dirs, &skipped_items);

    let _total_files = flattened_walk_file_data.len();
    let resolver: CachingResolver<TsConfigResolver<NodeModulesResolver>> =
        create_tsconfig_paths_resolver(&tsconfig);
    let mut file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>> =
        create_report_map_from_flattened_files(&flattened_walk_file_data);
    let mut files: Vec<GraphFile> = flattened_walk_file_data
        .par_iter_mut()
        .map(|file| {
            process_import_export_info(
                &mut file.import_export_info,
                &file.source_file_path,
                &resolver,
            );
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
    const MAX_ITERATIONS: i32 = 10_000_000;
    for i in 0..MAX_ITERATIONS {
        frontier = graph.bfs_step(frontier);

        if frontier.is_empty() {
            break;
        }
        if i >= MAX_ITERATIONS {
            return Err(JsErr::generic_failure(
                "exceeded max iterations".to_string(),
            ));
        }
    }

    let allow_list: Vec<glob::Pattern> = read_allow_list().map_err(JsErr::generic_failure)?;

    let reported_unused_files = BTreeMap::from_iter(
        graph
            .files
            .iter()
            .filter(|f| !f.1.is_used && !allow_list.iter().any(|p| p.matches(f.0))),
    );

    let unused_files_items: HashMap<String, Vec<ExportedItemReport>> = if report_exported_items {
        graph
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
            .collect()
    } else {
        HashMap::new()
    };

    Ok(UnusedFinderReport {
        unused_files: reported_unused_files
            .iter()
            .map(|(p, _)| p.to_string())
            .collect(),
        unused_files_items,
    })
}

// Looks in cwd for a file called `.unusedignore`
// allowed items can be:
// - specific file paths like `shared/internal/owa-react-hooks/src/useWhyDidYouUpdate.ts`
// - glob patterns (similar to a `.gitignore` file) `shared/internal/owa-datetime-formatters/**`
pub fn read_allow_list() -> Result<Vec<glob::Pattern>> {
    return match std::fs::read_to_string(".unusedignore") {
        Ok(list) => list
            .split("\n")
            .enumerate()
            .map(|(idx, line)| {
                glob::Pattern::new(line)
                    .map_err(|e| anyhow!("line {}: failed to parse pattern: {}", idx, e))
            })
            .collect::<Result<Vec<glob::Pattern>, anyhow::Error>>(),
        Err(e) => Err(anyhow!("failed to read .unusedignore file: {}", e)),
    };
}

pub fn process_import_export_info(
    f: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    process_executed_paths(f, source_file_path, resolver);
    process_async_imported_paths(f, source_file_path, resolver);
    process_exports_from(f, source_file_path, resolver);
    process_require_paths(f, source_file_path, resolver);
    process_import_path_ids(f, source_file_path, resolver);
}

#[cfg(test)]
mod test {
    use crate::FindUnusedItemsConfig;

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
            result.unwrap_err().message(),
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
            result.unwrap_err().message(),
            "regex parse error:\n    [A-Z.*\n    ^\nerror: unclosed character class"
        )
    }
}