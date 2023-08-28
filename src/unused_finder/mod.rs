pub mod node_visitor;
pub mod unused_finder_visitor_runner;
mod utils;

use napi_derive::napi;
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use crate::{
    file_extension::no_ext,
    import_resolver::TsconfigPathsJson,
    unused_finder::{
        node_visitor::ExportedItem,
        unused_finder_visitor_runner::ImportExportInfo,
        utils::{get_map_of_imports, retrieve_files, ResolvedItem},
    },
};

#[derive(Debug, PartialEq, Eq)]
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
}

pub fn find_unused_items(
    paths_to_read: Vec<String>,
    ts_config_path: String,
    skipped_dirs: Vec<String>,
    skipped_items: Vec<String>,
) -> Result<Vec<String>, crate::error::NapiLikeError> {
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

    let walked_files_map: HashMap<String, ImportExportInfo> = flattened_walk_file_data
        .drain(0..)
        .map(|f| {
            (
                no_ext(&f.source_file_path).to_string(),
                f.import_export_info,
            )
        })
        .collect();
    // HashMap wher key = the used file path, value = a hashset with the items imported from that file, note that those items could not belong to that file (for the cases of export from)
    let mut path_unused_items_map: HashMap<String, HashSet<ExportedItem>> = HashMap::new();

    walked_files_map
        .clone()
        .drain()
        .for_each(|(path, imp_exp_info)| {
            // Populate `unused_file_exports`
            path_unused_items_map.insert(path, imp_exp_info.exported_ids);
        });

    let mut unused_files: HashSet<_> = walked_files_map.keys().clone().into_iter().collect();

    let resolved_imports_map = get_map_of_imports(&tsconfig, &walked_files_map);
    resolved_imports_map
        .iter()
        .for_each(|(_path, imported_path_items_map)| {
            // Iterate over each file and mark the imported items as used in the origin file.
            imported_path_items_map
                .iter()
                .for_each(|(imported_path, imported_items)| {
                    if let Some(origin_file_exported_items) =
                        path_unused_items_map.get_mut(imported_path)
                    {
                        imported_items.iter().for_each(|item| {
                            match item {
                                ResolvedItem::Imported(imported) => {
                                    match imported {
                                        node_visitor::ImportedItem::ExecutionOnly
                                        | node_visitor::ImportedItem::Namespace => {
                                            // Even if exported elements are not imported specifically, side effects take place
                                            // In case of namespace import (import * as foo from 'foo'), an exhaustive search on which items of `foo` are used.
                                            // For now, assume that all items are used and by clearing the map we mark all items as used
                                            // TODO add node visitor that does search for specific items used.
                                            origin_file_exported_items.clear();
                                            unused_files.remove(imported_path);
                                        }
                                        _ => {
                                            unused_files.remove(imported_path);
                                            let i = ExportedItem::from(imported);
                                            origin_file_exported_items.remove(&i);
                                        }
                                    }
                                }
                                ResolvedItem::Exported(_) => {}
                            }
                        });
                    }
                });
        });
    let mut unused_items_len = 0;

    // Print the unused items for each file, sort them by file first
    let mut path_unused_items: Vec<_> = path_unused_items_map.into_iter().collect();
    path_unused_items.sort_by(|x, y| x.0.cmp(&y.0));
    path_unused_items.iter().for_each(|(k, v)| {
        if v.len() > 0 {
            if !unused_files.contains(k) {
                println!("From file {} found {:?} unused items", k, v);
            }
        }
        unused_items_len += v.len();
    });

    let results: Vec<String> = Vec::new();
    let mut unused_files: Vec<_> = unused_files.into_iter().collect();
    unused_files.sort();
    unused_files.iter().for_each(|f| {
        println!("Unused file: {}", f);
    });
    println!("Total unused files: {}", unused_files.len());
    println!("Total unused items: {}", unused_items_len);

    Ok(results)
}

#[cfg(test)]
mod test {
    use super::find_unused_items;

    #[test]
    fn test_error_in_glob() {
        let result = find_unused_items(
            vec!["tests/unused_finder".to_string()],
            "tests/unused_finder/tsconfig.json".to_string(),
            vec![".....///invalidpath****".to_string()],
            vec!["[A-Z].*".to_string(), "something".to_string()],
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().message,
            "wildcards are either regular `*` or recursive `**`"
        )
    }

    #[test]
    fn test_error_in_regex() {
        let result = find_unused_items(
            vec!["tests/unused_finder".to_string()],
            "tests/unused_finder/tsconfig.json".to_string(),
            vec![],
            vec!["[A-Z.*".to_string(), "something".to_string()],
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().message,
            "regex parse error:\n    [A-Z.*\n    ^\nerror: unclosed character class"
        )
    }
}
