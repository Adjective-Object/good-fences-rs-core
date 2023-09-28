mod export_collector_tests;
pub mod node_visitor;
pub mod unused_finder_visitor_runner;
mod utils;

use napi_derive::napi;
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
    sync::Arc,
};

use crate::{
    import_resolver::TsconfigPathsJson,
    unused_finder::{
        node_visitor::ImportedItem,
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
    let mut used_files: HashMap<String, &mut WalkFileMetaData> =
        HashMap::with_capacity(flattened_walk_file_data.len());

    let entry_files: Vec<WalkFileMetaData> = flattened_walk_file_data
        .iter_mut()
        .filter_map(|file| {
            if entry_packages.contains(&file.package_name) {
                let mut f = file.clone();
                process_import_export_info(&mut f, &tsconfig);
                return Some(f);
            }
            None
        })
        .collect();
    let mut unused_files: HashMap<String, &mut WalkFileMetaData> = flattened_walk_file_data
        .iter_mut()
        .filter_map(|f| {
            if f.source_file_path == "shared/internal/owa-service/src/contract/TokenResponse.ts" {
                dbg!(entry_packages.contains(&f.package_name));
            }
            if !entry_packages.contains(&f.package_name) {
                process_import_export_info(f, &tsconfig);
                return Some((f.source_file_path.clone(), f));
            }
            None
        })
        .collect();

    dbg!(unused_files.len());
    entry_files.iter().for_each(|file| {
        let ImportExportInfo {
            imported_path_ids,
            require_paths,
            imported_paths,
            export_from_ids,
            exported_ids: _,
            executed_paths,
        } = &file.import_export_info;
        for (key, values) in imported_path_ids.iter() {
            match unused_files.remove(key) {
                Some(r) => {
                    r.import_export_info
                        .exported_ids
                        .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                    used_files.insert(key.clone(), r);
                }
                None => {
                    if let Some(used_file) = used_files.get_mut(key) {
                        used_file
                            .import_export_info
                            .exported_ids
                            .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                    }
                }
            }
        }
        for (key, values) in export_from_ids {
            match unused_files.remove(key) {
                Some(r) => {
                    r.import_export_info
                        .exported_ids
                        .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                    used_files.insert(key.clone(), r);
                }
                None => {
                    if let Some(used_file) = used_files.get_mut(key) {
                        used_file
                            .import_export_info
                            .exported_ids
                            .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                    }
                }
            }
        }
        for key in require_paths {
            match unused_files.remove(key) {
                Some(r) => {
                    r.import_export_info.exported_ids.clear();
                    used_files.insert(key.clone(), r);
                }
                None => {}
            }
        }
        for key in imported_paths {
            match unused_files.remove(key) {
                Some(r) => {
                    r.import_export_info.exported_ids.clear();
                    used_files.insert(key.clone(), r);
                }
                None => {}
            }
        }
        for key in executed_paths {
            match unused_files.remove(key) {
                Some(r) => {
                    r.import_export_info.exported_ids.clear();
                    used_files.insert(key.clone(), r);
                }
                None => {}
            }
        }
        imported_path_ids.iter().for_each(|(path, items)| {
            let mut export_from_paths: HashMap<String, HashSet<ImportedItem>> = HashMap::new();
            {
                if let Some(used_file) = used_files.get_mut(path) {
                    used_file.import_export_info.exported_ids.retain(|i| !items.contains(&ImportedItem::from(i)));
                    export_from_paths = used_file.import_export_info.export_from_ids.iter().map(|(path, items)| {
                        // used_files.insert(path.clone(), unused_files.remove(path).unwrap());
                        (path.to_string(), items.clone())
                    }).collect();
                }
            }
            for (p, items) in export_from_paths {
                match unused_files.remove(&p) {
                    Some(removed) => {
                        for item in items {
                            match item {
                                ImportedItem::ExecutionOnly | ImportedItem::Namespace => {
                                    removed.import_export_info.exported_ids.clear();
                                },
                                _ => {
                                    removed.import_export_info.exported_ids.remove(&node_visitor::ExportedItem::from(&item));
                                }
                            }
                        }
                        used_files.insert(p, removed);
                    },
                    None => {
                        if let Some(used_file) = used_files.get_mut(&p) {
                            for item in items {
                                match item {
                                    ImportedItem::ExecutionOnly | ImportedItem::Namespace => {
                                        used_file.import_export_info.exported_ids.clear();
                                    },
                                    _ => {
                                        used_file.import_export_info.exported_ids.remove(&node_visitor::ExportedItem::from(&item));
                                    }
                                }
                            }
                        }
                    },
                } 
            }
        });
    
    });

    loop {
        let old_size = unused_files.len();
        let mut new_used_files: HashMap<String, &mut WalkFileMetaData> = HashMap::new();

        used_files.iter_mut().for_each(|(_used_file_path, file)| {
            let ImportExportInfo {
                imported_path_ids,
                require_paths,
                imported_paths,
                export_from_ids,
                exported_ids: _,
                executed_paths,
            } = &file.import_export_info;
            for (key, values) in imported_path_ids {
                match unused_files.remove(key) {
                    Some(r) => {
                        r.import_export_info
                            .exported_ids
                            .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                        new_used_files.insert(key.clone(), r);
                    }
                    None => {}
                }
            }
            for (key, values) in export_from_ids {
                match unused_files.remove(key) {
                    Some(r) => {
                        r.import_export_info
                            .exported_ids
                            .retain(|ids| !values.contains(&ImportedItem::from(ids)));
                        new_used_files.insert(key.clone(), r);
                    }
                    None => {}
                }
            }
            for key in require_paths {
                match unused_files.remove(key) {
                    Some(r) => {
                        new_used_files.insert(key.clone(), r);
                    }
                    None => {}
                }
            }
            for key in imported_paths {
                match unused_files.remove(key) {
                    Some(r) => {
                        new_used_files.insert(key.clone(), r);
                    }
                    None => {}
                }
            }
            for key in executed_paths {
                match unused_files.remove(key) {
                    Some(r) => {
                        new_used_files.insert(key.clone(), r);
                    }
                    None => {}
                }
            }
        });

        used_files.extend(new_used_files);

        if old_size == unused_files.len() {
            break;
        }
    }
    let results: Vec<String> = Vec::new();
    let unused_files = BTreeMap::from_iter(unused_files.iter());
    unused_files.iter().for_each(|f| {
        println!("\"{}\",", f.0);
    });
    println!("Total files: {}", total_files);
    println!("Total used files: {}", used_files.len());
    println!("Total unused files: {}", unused_files.len());

    Ok(results)
}

fn process_import_export_info(f: &mut WalkFileMetaData, tsconfig: &TsconfigPathsJson) {
    process_executed_paths(&mut f.import_export_info, tsconfig, &f.source_file_path);
    process_async_imported_paths(&mut f.import_export_info, tsconfig, &f.source_file_path);
    process_exports_from(&mut f.import_export_info, tsconfig, &f.source_file_path);
    process_require_paths(&mut f.import_export_info, tsconfig, &f.source_file_path);
    process_import_path_ids(&mut f.import_export_info, tsconfig, &f.source_file_path);
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
