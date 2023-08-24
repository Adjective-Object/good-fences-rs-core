pub mod node_visitor;
pub mod unused_finder_visitor_runner;
mod utils;

use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    iter::FromIterator,
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

    let mut walked_files_map: HashMap<String, ImportExportInfo> = flattened_walk_file_data
        .drain(0..)
        .map(|f| {
            (
                no_ext(&f.source_file_path).to_string(),
                f.import_export_info,
            )
        })
        .collect();

    // HashMap wher key = the used file path, value = a hashset with the items imported from that file, note that those items could not belong to that file (for the cases of export from)
    let mut unused_file_exports: HashMap<&String, HashSet<ExportedItem>> = HashMap::new();

    let resolved_imports_map = get_map_of_imports(&tsconfig, &walked_files_map);
    resolved_imports_map
        .iter()
        .for_each(|(_f, path_items_map)| {
            path_items_map.iter().for_each(|(p, items)| {
                if let Some(used_items) = unused_file_exports.get_mut(p) {
                    for item in items.iter() {
                        match item {
                            ResolvedItem::Imported(imported_item) => {
                                used_items.insert(imported_item.into());
                            }
                            ResolvedItem::Exported(exported_item) => {
                                used_items.insert(exported_item.clone());
                            }
                        }
                    }
                } else {
                    unused_file_exports.insert(
                        p,
                        HashSet::from_iter(items.iter().filter_map(|i| match i {
                            ResolvedItem::Imported(imported) => return Some(imported.into()),
                            ResolvedItem::Exported(exported) => return Some(exported.clone()),
                        })),
                    );
                }
            });
        });

    // Get unused items from used files:
    // Compare the used items with ImportExportInfo and withold the unused items
    let mut is_executed = false;
    unused_file_exports
        .iter()
        .for_each(|(used_path, used_items)| {
            is_executed = false;
            if let Some(found_file) = walked_files_map.get_mut(*used_path) {
                used_items.iter().for_each(|used_item| {
                    match &used_item {
                        // Remove from 'exported_ids' the items that were used if named or default
                        ExportedItem::Default | ExportedItem::Named(_) => {
                            if !found_file.exported_ids.remove(used_item) {
                                return;
                            }
                        }
                        ExportedItem::Namespace => {
                            found_file
                                .exported_ids
                                .retain(|k| &ExportedItem::Default == k);
                        }
                        ExportedItem::ExecutionOnly => {
                            is_executed = true;
                        }
                    }
                });
                // TODO resolve export from ids and mark them as used
            }
            if is_executed {
                walked_files_map.remove(*used_path);
            }
        });

    let mut unused_items_len = 0;

    // Print the unused items for each file
    let mut vec_keys: Vec<&&String> = unused_file_exports.keys().into_iter().collect();
    vec_keys.sort();
    let mut results: Vec<String> = Vec::new();
    for key in vec_keys {
        if let Some(used_file) = walked_files_map.remove(*key) {
            if !used_file.exported_ids.is_empty() {
                unused_items_len += used_file.exported_ids.len();
                results.push(
                    format!(
                        "From file {} found {:?} unused items",
                        key, used_file.exported_ids
                    )
                    .to_string(),
                );
                println!(
                    "From file {} found {:?} unused items",
                    key, used_file.exported_ids
                );
            }
        }
    }
    println!("Total unused files: {}", walked_files_map.keys().len());
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
