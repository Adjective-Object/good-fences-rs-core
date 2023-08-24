use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::Arc;

use jwalk::WalkDirGeneric;
use path_slash::PathExt;
use relative_path::RelativePath;

use crate::import_resolver::{resolve_ts_import, ResolvedImport, TsconfigPathsJson};

use super::node_visitor::{ExportedItem, ImportedItem};
use super::unused_finder_visitor_runner::{get_import_export_paths_map, ImportExportInfo};
use super::{WalkFileMetaData, WalkedFile};

#[derive(Debug, Hash, PartialEq, Eq)]
pub enum ResolvedItem {
    Imported(ImportedItem),
    Exported(ExportedItem),
}

impl From<ImportedItem> for ResolvedItem {
    fn from(item: ImportedItem) -> Self {
        ResolvedItem::Imported(item)
    }
}

impl From<ExportedItem> for ResolvedItem {
    fn from(item: ExportedItem) -> Self {
        ResolvedItem::Exported(item)
    }
}

/**
 * transform files to know exactly what items are being imported
 * key - String: Source file path
 * value - Hashmap with keys: the resolved imported paths from the outer key file, value: the items imported from the key path
 */
pub fn get_map_of_imports(
    tsconfig: &TsconfigPathsJson,
    walked_files: &HashMap<String, ImportExportInfo>,
) -> HashMap<String, HashMap<String, HashSet<ResolvedItem>>> {
    let complex_hash: HashMap<String, HashMap<String, HashSet<ResolvedItem>>> = walked_files
        .iter()
        .map(|(source_file_path, import_export_info)| {
            let mut resolved_map: HashMap<String, HashSet<ResolvedItem>> = HashMap::new();
            import_export_info.require_paths.iter().for_each(|path| {
                if let Ok(resolved) = resolve_ts_import(
                    &tsconfig,
                    &RelativePath::new(&source_file_path.to_string()),
                    path,
                ) {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = if resolved.is_dir() {
                            resolved.join("index").to_slash().unwrap().to_string()
                        } else {
                            resolved.to_slash().unwrap().to_string()
                        };
                        if let Some(items) = resolved_map.get_mut(&slashed) {
                            items.insert(ExportedItem::Default.into());
                        } else {
                            resolved_map.insert(
                                slashed.clone(),
                                HashSet::from_iter(vec![ExportedItem::Default.into()]),
                            );
                        }
                    }
                }
            });
            import_export_info.imported_paths.iter().for_each(|path| {
                if let Ok(resolved) = resolve_ts_import(
                    &tsconfig,
                    &RelativePath::new(&source_file_path.to_string()),
                    path,
                ) {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = if resolved.is_dir() {
                            resolved.join("index").to_slash().unwrap().to_string()
                        } else {
                            resolved.to_slash().unwrap().to_string()
                        };
                        if let Some(items) = resolved_map.get_mut(&slashed) {
                            items.insert(ExportedItem::Default.into());
                        } else {
                            resolved_map.insert(
                                slashed.clone(),
                                HashSet::from_iter(vec![ExportedItem::ExecutionOnly.into()]),
                            );
                        }
                    }
                }
            });
            import_export_info.export_from_ids.iter().for_each(
                |(imported_path, imported_items)| {
                    if let Ok(resolved) = resolve_ts_import(
                        &tsconfig,
                        &RelativePath::new(&source_file_path.to_string()),
                        &imported_path,
                    ) {
                        if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                            let slashed = if resolved.is_dir() {
                                resolved.join("index").to_slash().unwrap().to_string()
                            } else {
                                resolved.to_slash().unwrap().to_string()
                            };
                            if let Some(items) = resolved_map.get_mut(&slashed) {
                                for ie in imported_items {
                                    items.insert(ie.to_owned().into());
                                }
                            } else {
                                resolved_map.insert(
                                    slashed.clone(),
                                    imported_items.iter().map(|i| i.clone().into()).collect(),
                                );
                            }
                        }
                    }
                },
            );
            import_export_info.imported_path_ids.iter().for_each(
                |(imported_path, imported_items)| {
                    if let Ok(resolved) = resolve_ts_import(
                        &tsconfig,
                        &RelativePath::new(&source_file_path.to_string()),
                        &imported_path,
                    ) {
                        if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                            let slashed = if resolved.is_dir() {
                                resolved.join("index").to_slash().unwrap().to_string()
                            } else {
                                resolved.to_slash().unwrap().to_string()
                            };
                            if let Some(items) = resolved_map.get_mut(&slashed) {
                                for ie in imported_items {
                                    items.insert(ie.to_owned().into());
                                }
                            } else {
                                resolved_map.insert(
                                    slashed.clone(),
                                    imported_items.iter().map(|i| i.clone().into()).collect(),
                                );
                            }
                        }
                    }
                },
            );

            (source_file_path.clone(), resolved_map)
        })
        .collect();
    return complex_hash;
}

pub fn retrieve_files(
    start_path: &str,
    skipped_dirs: Option<Vec<glob::Pattern>>,
    skipped_items: Arc<Vec<regex::Regex>>,
) -> Vec<WalkedFile> {
    let walk_dir = WalkDirGeneric::<(String, WalkedFile)>::new(start_path).process_read_dir(
        move |dir_state, children| {
            children.iter_mut().for_each(|dir_entry_res| {
                if let Ok(dir_entry) = dir_entry_res {
                    if dir_entry.file_type().is_dir()
                        && (dir_entry.file_name() == "node_modules"
                            || dir_entry.file_name() == "lib")
                    {
                        dir_entry.read_children_path = None;
                    }
                }
            });
            children.retain(|dir_entry_result| match dir_entry_result {
                Ok(dir_entry) => {
                    return should_retain_dir_entry(dir_entry, &skipped_dirs);
                }
                Err(_) => todo!(),
            });

            children.iter_mut().for_each(|dir_entry_result| {
                if let Ok(dir_entry) = dir_entry_result {
                    if dir_entry.file_name() == "package.json" {
                        match std::fs::read_to_string(dir_entry.path()) {
                            Ok(text) => {
                                let pkg_json: serde_json::Value =
                                    serde_json::from_str(&text).unwrap();
                                let name = pkg_json["name"].as_str();
                                if let Some(name) = name {
                                    *dir_state = name.to_string();
                                }
                            }
                            Err(_) => {} // invalid package.json file
                        }
                    }
                }
            });

            children.iter_mut().for_each(|child_result| {
                match child_result {
                    Ok(dir_entry) => {
                        match dir_entry.file_name.to_str() {
                            Some(file_name) => {
                                if dir_entry.file_type.is_dir() {
                                    return;
                                }
                                // Source file [.ts, .tsx, .js, .jsx]
                                let joined = &dir_entry.parent_path.join(file_name);
                                let slashed = joined.to_slash();
                                let slashed = match slashed {
                                    Some(slashed) => slashed,
                                    None => todo!(),
                                };
                                let visitor_result = get_import_export_paths_map(
                                    slashed.to_string(),
                                    skipped_items.clone(),
                                );
                                match visitor_result {
                                    Ok(import_export_info) => {
                                        dir_entry.client_state =
                                            WalkedFile::SourceFile(WalkFileMetaData {
                                                package_name: dir_state.clone(),
                                                import_export_info,
                                                source_file_path: dir_entry
                                                    .path()
                                                    .to_slash()
                                                    .unwrap()
                                                    .to_string(),
                                            });
                                    }
                                    Err(_) => todo!(),
                                }
                            }
                            None => todo!(),
                        }
                    }
                    Err(_) => todo!(),
                }
            });
        },
    );
    walk_dir
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(e) => return Some(e.client_state),
            Err(_) => None,
        })
        .collect()
}

fn should_retain_dir_entry(
    dir_entry: &jwalk::DirEntry<(String, WalkedFile)>,
    skip_dirs: &Option<Vec<glob::Pattern>>,
) -> bool {
    match dir_entry.path().to_slash() {
        Some(slashed) => {
            if dir_entry.file_type.is_dir() {
                if slashed.ends_with("/node_modules") {
                    return false; // Return false ignores dir or files
                }
                if let Some(skips) = &skip_dirs {
                    if skips.iter().any(|skip| skip.matches(slashed.as_ref())) {
                        return false;
                    }
                }
                return true;
            }
            if dir_entry.file_type.is_file() {
                return is_js_ts_file(slashed.as_ref());
            }
        }
        _ => return false,
    }
    return false;
}

fn is_js_ts_file(s: &str) -> bool {
    s.ends_with(".ts") || s.ends_with(".tsx") || s.ends_with(".js") || s.ends_with(".jsx")
}
