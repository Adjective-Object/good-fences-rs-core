pub mod node_visitor;
pub mod unused_finder_visitor_runner;
mod utils;

use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    iter::FromIterator,
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
    skipped: Vec<String>,
) -> Result<Vec<String>, crate::error::NapiLikeError> {
    let tsconfig = match TsconfigPathsJson::from_path(ts_config_path) {
        Ok(tsconfig) => tsconfig,
        Err(e) => panic!("Unable to read tsconfig file: {}", e),
    };
    let skipped = skipped.iter().map(|s| glob::Pattern::new(s));
    let skipped: Arc<Vec<glob::Pattern>> = match skipped.into_iter().collect() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            return Err(crate::error::NapiLikeError {
                status: napi::Status::InvalidArg,
                message: e.msg.to_string(),
            })
        }
    };
    // Walk on all files and retrieve the WalkFileData from them
    let mut flattened_walk_file_data: Vec<WalkFileMetaData> = paths_to_read
        .par_iter()
        .map(|path| {
            let mut walked_files = retrieve_files(path, Some(skipped.to_vec()));
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
            vec![".".to_string()],
            "tests/unused_finder/tsconfig.json".to_string(),
            vec![".....///invalidpath****".to_string()],
        );
        dbg!(&result);
        assert!(result.is_err());
    }
}
