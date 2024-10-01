use crate::parse::{get_import_export_paths_map, ExportKind, ImportExportInfo, ImportedItem};
use crate::walked_file::{UnusedFinderSourceFile, WalkedFile};
use anyhow::{Error, Result};
use import_resolver::manual_resolver::{resolve_with_extension, ResolvedImport};
use jwalk::WalkDirGeneric;
use packagejson::PackageJson;
use path_slash::PathBufExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use swc_core::common::FileName;
use swc_core::ecma::loader::resolve::Resolve;

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

// shared logic for "process_*" methods that maps a collection into another colleciton,
// resolving the import paths of each of the elements.
fn resolve_imports_collection<TCollection, TCollectionOut, T>(
    source_file_path: &str,
    resolver: &dyn Resolve,
    collection: TCollection,
    import_from_item: for<'a> fn(&'a T) -> &'a str,
    update_item: for<'a> fn(T, String) -> T,
) -> Result<TCollectionOut, Error>
where
    TCollection: IntoIterator<Item = T>,
    TCollectionOut: FromIterator<T>,
{
    collection
        .into_iter()
        .filter_map(|v| {
            let imported_path = import_from_item(&v);

            match resolve_with_extension(
                FileName::Real(source_file_path.into()),
                imported_path,
                resolver,
            ) {
                Ok(ResolvedImport::ProjectLocalImport(resolved)) => {
                    let slashed = resolved.to_slash().unwrap().to_string();
                    Some(Ok(update_item(v, slashed)))
                }
                Err(e) => Some(Err(e)),
                _ => None,
            }
        })
        .collect()
}

// import foo, {bar as something} from './foo'`
pub fn process_import_path_ids(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<(), Error> {
    let res: Result<HashMap<String, _>, _> = resolve_imports_collection(
        source_file_path,
        resolver,
        import_export_info.imported_path_ids.drain(),
        for<'a> |(a, _): &'a (String, HashSet<ImportedItem>)| -> &'a str { a },
        |(_, names), resolved| (resolved, names),
    );

    res.map(|res| {
        // on success, update the import_export_info.
        // Otherwise, hide Ok() value and return the error.
        import_export_info.imported_path_ids = res;
    })
}

// `export {default as foo, bar} from './foo'`
pub fn process_exports_from(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<(), Error> {
    let res: Result<HashMap<String, _>, _> = resolve_imports_collection(
        source_file_path,
        resolver,
        import_export_info.export_from_ids.drain(),
        for<'a> |(a, _): &'a (String, HashSet<ImportedItem>)| -> &'a str { a },
        |(_, names), resolved| (resolved, names),
    );

    res.map(|res| {
        // on success, update the import_export_info.
        // Otherwise, hide Ok() value and return the error.
        import_export_info.export_from_ids = res;
    })
}

// import('./foo')
pub fn process_async_imported_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<(), Error> {
    let res: Result<HashSet<String>, _> = resolve_imports_collection(
        source_file_path,
        resolver,
        import_export_info.imported_paths.drain(),
        for<'a> |a: &'a String| -> &'a str { a },
        |_, resolved| resolved,
    );

    res.map(|res| {
        // on success, update the import_export_info.
        // Otherwise, hide Ok() value and return the error.
        import_export_info.imported_paths = res;
    })
}

// import './foo'
pub fn process_executed_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<(), Error> {
    let res: Result<HashSet<String>, _> = resolve_imports_collection(
        source_file_path,
        resolver,
        import_export_info.executed_paths.drain(),
        for<'a> |a: &'a String| -> &'a str { a },
        |_, resolved| resolved,
    );

    res.map(|res| {
        // on success, update the import_export_info.
        // Otherwise, hide Ok() value and return the error.
        import_export_info.executed_paths = res;
    })
}

// require('foo')
pub fn process_require_paths(
    import_export_info: &mut ImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<(), Error> {
    let res: Result<HashMap<String, _>, _> = resolve_imports_collection(
        source_file_path,
        resolver,
        import_export_info.export_from_ids.drain(),
        for<'a> |(a, _): &'a (String, HashSet<ImportedItem>)| -> &'a str { a },
        |(_, names), resolved| (resolved, names),
    );

    res.map(|res| {
        // on success, update the import_export_info.
        // Otherwise, hide Ok() value and return the error.
        import_export_info.export_from_ids = res;
    })
}

pub fn retrieve_files(
    start_path: &str,
    skipped_dirs: Option<Vec<glob::Pattern>>,
    skipped_items: Arc<Vec<regex::Regex>>,
) -> Vec<WalkedFile> {
    let visitor = UnusedFinderWalkVisitor::new(skipped_dirs, skipped_items);
    let walk_dir =
        WalkDirGeneric::<(String, WalkedFile)>::new(start_path).process_read_dir(
            move |dir_state,
                  children: &mut Vec<
                Result<jwalk::DirEntry<(String, WalkedFile)>, jwalk::Error>,
            >| { visitor.visit_directory(dir_state, children) },
        );
    walk_dir
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(e) => Some(e.client_state),
            Err(_) => None,
        })
        .collect()
}

// Visitor during a directory walk that collects information about source files
// for the unused finder
struct UnusedFinderWalkVisitor {
    skipped_dirs: Option<Vec<glob::Pattern>>,
    skipped_items: Arc<Vec<regex::Regex>>,
}

impl UnusedFinderWalkVisitor {
    pub fn new(
        skipped_dirs: Option<Vec<glob::Pattern>>,
        skipped_items: Arc<Vec<regex::Regex>>,
    ) -> Self {
        UnusedFinderWalkVisitor {
            skipped_dirs,
            skipped_items,
        }
    }

    // callback meant to be called during a file walk of a directory
    // (e.g. with jwalk's process_read_dir() callback)
    pub fn visit_directory(
        &self,
        current_package_name: &mut String,
        children: &mut Vec<jwalk::Result<jwalk::DirEntry<(String, WalkedFile)>>>,
    ) {
        children.iter_mut().for_each(|dir_entry_res| {
            if let Ok(dir_entry) = dir_entry_res {
                if dir_entry.file_name() == "node_modules" || dir_entry.file_name() == "lib" {
                    dir_entry.read_children_path = None;
                }
            }
        });

        // if there is a package.json file, we can use it to get the package name
        // This should be done in a separate iteration _before_ visiting the rest of the files
        // in the folder, because package.json files influence their peer files.
        if let Some(package_json) = children
            .iter()
            .filter_map(|f| match f {
                Ok(f) => Some(f),
                Err(_) => None,
            })
            .find(|entry| entry.file_name() == "package.json")
        {
            let file = std::fs::File::open(package_json.path()).unwrap();
            let pkg_json: PackageJson = serde_json::from_reader(file).unwrap();
            if let Some(name) = pkg_json.name {
                *current_package_name = name.to_string();
            }
        }

        children.retain(|dir_entry_result| match dir_entry_result {
            Ok(dir_entry) => should_retain_dir_entry(dir_entry, &self.skipped_dirs),
            Err(_) => false,
        });

        for dir_entry in children {
            let child = match dir_entry {
                Ok(ref mut dir_entry) => dir_entry,
                Err(_) => continue,
            };

            if child.file_type.is_dir() {
                continue;
            }

            let file_name = match child.file_name.to_str() {
                Some(name) => name,
                None => continue,
            };

            // Source file [.ts, .tsx, .js, .jsx]
            let joined = &child.parent_path.join(file_name);
            let slashed = joined.to_slash().unwrap();
            let visitor_result = get_import_export_paths_map(
                slashed.to_string(),
                // note: this clone() is cloning the Arc<> pointer, not the data the Arc references
                // See: https://doc.rust-lang.org/std/sync/struct.Arc.html
                self.skipped_items.clone(),
            );
            if let Ok(import_export_info) = visitor_result {
                child.client_state = WalkedFile::SourceFile(Box::new(UnusedFinderSourceFile {
                    package_name: current_package_name.clone(),
                    import_export_info,
                    source_file_path: child.path().to_slash().unwrap().to_string(),
                }));
            }
        }
    }
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
    dir_entry.file_type().is_dir()
}

fn is_js_ts_file(s: &str) -> bool {
    s.ends_with(".ts") || s.ends_with(".tsx") || s.ends_with(".js") || s.ends_with(".jsx")
}
