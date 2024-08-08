use std::sync::Arc;

use jwalk::WalkDirGeneric;
use path_slash::PathBufExt;
use swc_core::common::FileName;
use swc_core::ecma::loader::resolve::Resolve;

use import_resolver::{resolve_with_extension, ResolvedImport};

use super::node_visitor::{ExportKind, ImportedItem};
use super::unused_finder_visitor_runner::{get_import_export_paths_map, ImportExportInfo};
use super::{SourceFile, WalkedFile};

#[derive(Debug, Hash, PartialEq, Eq)]
pub enum ResolvedItem {
    Imported(ImportedItem),
    Exported(ExportKind),
}

impl From<ImportedItem> for ResolvedItem {
    fn from(item: ImportedItem) -> Self {
        ResolvedItem::Imported(item)
    }
}

impl From<ExportKind> for ResolvedItem {
    fn from(item: ExportKind) -> Self {
        ResolvedItem::Exported(item)
    }
}

// import foo, {bar as something} from './foo'`
pub fn process_import_path_ids(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    import_export_info.imported_path_ids = import_export_info
        .imported_path_ids
        .drain()
        .filter_map(|(imported_path, imported_items)| {
            match resolve_with_extension(
                FileName::Real(source_file_path.clone().into()),
                &imported_path,
                resolver,
            ) {
                Ok(resolved) => {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = resolved.to_slash().unwrap().to_string();
                        return Some((slashed, imported_items));
                    }
                }
                Err(_e) => return None,
            }
            None
        })
        .collect();
}

// `export {default as foo, bar} from './foo'`
pub fn process_exports_from(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    import_export_info.export_from_ids = import_export_info
        .export_from_ids
        .drain()
        .filter_map(|(imported_path, imported_items)| {
            // match resolve_ts_import(tsconfig_paths, initial_path, raw_import_path)
            match resolve_with_extension(
                FileName::Real(source_file_path.clone().into()),
                &imported_path,
                resolver,
            ) {
                Ok(resolved) => {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = resolved.to_slash().unwrap().to_string();

                        return Some((slashed, imported_items));
                    }
                }
                Err(_e) => {}
            }

            None
        })
        .collect();
}

// import('./foo')
pub fn process_async_imported_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    import_export_info.imported_paths = import_export_info
        .imported_paths
        .drain()
        .filter_map(|imported_path| {
            match resolve_with_extension(
                FileName::Real(source_file_path.clone().into()),
                &imported_path,
                resolver,
            ) {
                Ok(resolved) => {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = resolved.to_slash().unwrap().to_string();

                        return Some(slashed);
                    }
                }
                Err(_e) => {}
            }
            None
        })
        .collect();
}

// import './foo'
pub fn process_executed_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    import_export_info.executed_paths = import_export_info
        .executed_paths
        .drain()
        .filter_map(|executed_path| {
            match resolve_with_extension(
                FileName::Real(source_file_path.clone().into()),
                &executed_path,
                resolver,
            ) {
                Ok(resolved) => {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = resolved.to_slash().unwrap().to_string();

                        return Some(slashed);
                    }
                }
                Err(_e) => {}
            }

            None
        })
        .collect();
}

// require('foo')
pub fn process_require_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &String,
    resolver: &dyn Resolve,
) {
    import_export_info.require_paths = import_export_info
        .require_paths
        .drain()
        .filter_map(|required_path| {
            match resolve_with_extension(
                FileName::Real(source_file_path.clone().into()),
                &required_path,
                resolver,
            ) {
                Ok(resolved) => {
                    if let ResolvedImport::ProjectLocalImport(resolved) = resolved {
                        let slashed = resolved.to_slash().unwrap().to_string();

                        return Some(slashed);
                    }
                }
                Err(_e) => return None,
            }
            None
        })
        .collect();
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
                    if dir_entry.file_name() == "node_modules" || dir_entry.file_name() == "lib" {
                        dir_entry.read_children_path = None;
                    }
                }
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
            children.retain(|dir_entry_result| match dir_entry_result {
                Ok(dir_entry) => should_retain_dir_entry(dir_entry, &skipped_dirs),
                Err(_) => false,
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
                                let slashed = joined.to_slash().unwrap();
                                let visitor_result = get_import_export_paths_map(
                                    slashed.to_string(),
                                    skipped_items.clone(),
                                );
                                match visitor_result {
                                    Ok(import_export_info) => {
                                        dir_entry.client_state =
                                            WalkedFile::SourceFile(SourceFile {
                                                package_name: dir_state.clone(),
                                                import_export_info,
                                                source_file_path: dir_entry
                                                    .path()
                                                    .to_slash()
                                                    .unwrap()
                                                    .to_string(),
                                            });
                                    }
                                    Err(_) => {}
                                }
                            }
                            None => return,
                        }
                    }
                    Err(_) => {}
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
            if let Some(skips) = &skip_dirs {
                if skips.iter().any(|skip| skip.matches(slashed.as_ref())) {
                    return false;
                }
            }

            if dir_entry.file_type.is_file() {
                if !slashed.contains("/src/") {
                    return false;
                }
                return is_js_ts_file(slashed.as_ref());
            }
        }
        _ => return false,
    }
    return dir_entry.file_type().is_dir();
}

fn is_js_ts_file(s: &str) -> bool {
    s.ends_with(".ts") || s.ends_with(".tsx") || s.ends_with(".js") || s.ends_with(".jsx")
}
