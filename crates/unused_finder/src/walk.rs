use crate::logger::{Logger, StdioLogger};
use crate::parse::get_file_import_export_info;
use crate::walked_file::{WalkedFile, WalkedPackage, WalkedSourceFile};
use jwalk::WalkDirGeneric;
use path_slash::PathBufExt;
use rayon::iter::Either;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Debug)]
pub struct WalkFileResult {
    // Map of package name to (path, package.json)
    pub packages: Vec<WalkedPackage>,
    // Map of source file path to source file data
    pub source_files: Vec<WalkedSourceFile>,
}

/// Walks the root paths of a project and returns a list of source files and packages
pub fn walk_src_files(
    logger: impl Logger,
    root_paths: &Vec<String>,
    skipped_dirs: &Arc<Vec<glob::Pattern>>,
    skipped_items: &Arc<Vec<regex::Regex>>,
) -> WalkFileResult {
    let (source_files, packages): (Vec<WalkedSourceFile>, Vec<WalkedPackage>) = root_paths
        .par_iter()
        .map(|path| {
            jwalk_src_subtree(
                logger,
                path,
                Some(skipped_dirs.to_vec()),
                skipped_items.clone(),
            )
        })
        .flatten()
        .partition_map(
            |file: WalkedFile| -> Either<WalkedSourceFile, WalkedPackage> {
                match file {
                    WalkedFile::SourceFile(file) => Either::Left(file),
                    WalkedFile::PackageJson(walked_pkg) => Either::Right(walked_pkg),
                }
            },
        );

    WalkFileResult {
        packages,
        source_files,
    }
}

pub fn jwalk_src_subtree(
    logger: impl Logger,
    start_path: &str,
    skipped_dirs: Option<Vec<glob::Pattern>>,
    skipped_items: Arc<Vec<regex::Regex>>,
) -> Vec<WalkedFile> {
    let visitor = UnusedFinderWalkVisitor::new(skipped_dirs, skipped_items);
    let walk_dir = WalkDirGeneric::<(Option<String>, Option<WalkedFile>)>::new(start_path)
        .process_read_dir(move |dir_state, children| {
            visitor.visit_directory(&StdioLogger {}, dir_state, children)
        });
    walk_dir
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(e) => e.client_state,
            Err(e) => {
                logger.log(format!("error during walkdir: {e}"));
                None
            }
        })
        .collect()
}

// Visitor during a directory walk that collects information about source files
// for the unused finder
struct UnusedFinderWalkVisitor {
    skipped_dirs: Option<Vec<glob::Pattern>>,
    skipped_items: Arc<Vec<regex::Regex>>,
}

type UnusedFinderDirState = Option<String>;
type UnusedFinderFileState = Option<WalkedFile>;
type UnusedFinderWalkDirEntry = jwalk::DirEntry<(UnusedFinderDirState, UnusedFinderFileState)>;

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
        logger: impl Logger,
        directory_state: &mut Option<String>,
        children: &mut Vec<jwalk::Result<UnusedFinderWalkDirEntry>>,
    ) {
        children.iter_mut().for_each(|dir_entry_res| {
            // drop node_modules and lib directories from iteration
            if let Ok(dir_entry) = dir_entry_res {
                if dir_entry.file_name() == "node_modules" || dir_entry.file_name() == "lib" {
                    dir_entry.read_children_path = None;
                }
            }

            // record package.json files, if any
            if let Ok(dir_entry) = dir_entry_res {
                if dir_entry.file_name() == "package.json" {
                    let walked_package = match WalkedPackage::from_path(dir_entry.path()) {
                        Ok(pkg) => pkg,
                        Err(e) => {
                            logger.log(format!("Error reading package.json: {:?}", e));
                            return;
                        }
                    };
                    *directory_state = walked_package.package_json.name.clone();
                    dir_entry.client_state = Some(WalkedFile::PackageJson(walked_package));
                }
            }
        });

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
            let visitor_result = get_file_import_export_info(
                joined,
                // note: this clone() is cloning the Arc<> pointer, not the data the Arc references
                // See: https://doc.rust-lang.org/std/sync/struct.Arc.html
                self.skipped_items.clone(),
            );
            if let Ok(import_export_info) = visitor_result {
                child.client_state = Some(WalkedFile::SourceFile(WalkedSourceFile {
                    owning_package: directory_state.clone(),
                    import_export_info,
                    source_file_path: child.path(),
                }));
            }
        }
    }
}

fn should_retain_dir_entry<T: jwalk::ClientState>(
    dir_entry: &jwalk::DirEntry<T>,
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
