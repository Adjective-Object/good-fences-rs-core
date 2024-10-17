use crate::ignore_file::IgnoreFile;
use crate::logger::{Logger, StdioLogger};
use crate::parse::get_file_import_export_info;
use crate::walked_file::{WalkedFile, WalkedPackage, WalkedSourceFile};
use jwalk::WalkDirGeneric;
use path_slash::PathBufExt;
use rayon::prelude::*;
use std::ffi::OsStr;
use std::sync::Arc;

#[derive(Debug)]
pub struct WalkFileResult {
    // List of walked packages
    pub packages: Vec<WalkedPackage>,
    // List of walked source files
    pub source_files: Vec<WalkedSourceFile>,
    // List of files to ignore unused symbols in entirely
    pub ignore_files: Vec<IgnoreFile>,
}

/// Walks the root paths of a project and returns a list of source files and packages
pub fn walk_src_files(
    logger: impl Logger,
    root_paths: &Vec<String>,
    skipped_dirs: &Arc<Vec<glob::Pattern>>,
) -> WalkFileResult {
    // run the parallel walk
    let visited = root_paths
        .par_iter()
        .map(|path| jwalk_src_subtree(logger, path, skipped_dirs.to_vec()))
        .flatten()
        .collect::<Vec<WalkedFile>>();

    // partition the results
    let mut source_files: Vec<WalkedSourceFile> = Vec::new();
    let mut packages: Vec<WalkedPackage> = Vec::new();
    let mut ignore_files: Vec<IgnoreFile> = Vec::new();
    for file in visited {
        match file {
            WalkedFile::SourceFile(file) => source_files.push(file),
            WalkedFile::PackageJson(file) => packages.push(file),
            WalkedFile::IgnoreFile(file) => ignore_files.push(file),
        }
    }

    WalkFileResult {
        packages,
        source_files,
        ignore_files,
    }
}

pub fn jwalk_src_subtree(
    logger: impl Logger,
    start_path: &str,
    skipped_dirs: Vec<glob::Pattern>,
) -> Vec<WalkedFile> {
    let visitor = UnusedFinderWalkVisitor::new(skipped_dirs);
    let walk_dir = WalkDirGeneric::<(Option<String>, Option<WalkedFile>)>::new(start_path)
        .process_read_dir(move |dir_state, children| {
            visitor.visit_directory(&StdioLogger {}, dir_state, children)
        });
    walk_dir
        .into_iter()
        .filter_map(
            |entry: Result<UnusedFinderWalkDirEntry, jwalk::Error>| match entry {
                Ok(e) => e.client_state,
                Err(e) => {
                    logger.log(format!("error during walkdir: {e}"));
                    None
                }
            },
        )
        .collect()
}

// Visitor during a directory walk that collects information about source files
// for the unused finder
struct UnusedFinderWalkVisitor {
    // Names of directories we skip during the walk.
    //
    // These can either match the full path of the directory, or just the directory name.
    skipped_dirs: Vec<glob::Pattern>,
}

type UnusedFinderDirState = Option<String>;
type UnusedFinderFileState = Option<WalkedFile>;
type UnusedFinderWalkDirEntry = jwalk::DirEntry<(UnusedFinderDirState, UnusedFinderFileState)>;

impl UnusedFinderWalkVisitor {
    pub fn new(skipped_dirs: Vec<glob::Pattern>) -> Self {
        UnusedFinderWalkVisitor { skipped_dirs }
    }

    // callback meant to be called during a file walk of a directory
    // (e.g. with jwalk's process_read_dir() callback)
    pub fn visit_directory(
        &self,
        logger: impl Logger,
        directory_state: &mut Option<String>,
        children: &mut [jwalk::Result<UnusedFinderWalkDirEntry>],
    ) {
        // Log read errors for each child, and partition into a list of files & a list
        // of directories
        let (dirs, mut files): (
            Vec<&mut UnusedFinderWalkDirEntry>,
            Vec<&mut UnusedFinderWalkDirEntry>,
        ) = children
            .iter_mut()
            .filter_map(|x| match x {
                Ok(e) => Some(e),
                Err(e) => {
                    logger.log(format!("error during walkdir: {e}"));
                    None
                }
            })
            .partition(|f| f.file_type.is_dir());

        // filter out directories we should skip
        dirs.into_iter().for_each(|f| {
            if !self.should_walk_dir(f) {
                f.read_children_path = None;
            }
        });

        // first, set the dir state from the package.json file
        files.iter_mut().for_each(|dir_entry| {
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
        });

        // then, process each file in the directory
        files.par_iter_mut().for_each(|file| {
            if file.file_name() == ".unusedignore" {
                // also parse unused files during this pass
                let ignore_file = match IgnoreFile::read(file.path()) {
                    Ok(ignore_file) => ignore_file,
                    Err(e) => {
                        logger.log(format!("Error reading .ignore file: {:?}", e));
                        return;
                    }
                };
                file.client_state = Some(WalkedFile::IgnoreFile(ignore_file));
            } else if is_js_ts_file(file.file_name()) {
                // Source file [.ts, .tsx, .js, .jsx]
                let visitor_result = get_file_import_export_info(&file.path());
                match visitor_result {
                    Ok(import_export_info) => {
                        file.client_state = Some(WalkedFile::SourceFile(WalkedSourceFile {
                            owning_package: directory_state.clone(),
                            import_export_info,
                            source_file_path: file.path(),
                        }))
                    }
                    Err(e) => {
                        logger.log(format!(
                            "Error reading/parsing source file {:?}: {:?}",
                            file.path(),
                            e
                        ));
                    }
                }
            }
        });
    }

    fn should_walk_dir(&self, dir_entry: &UnusedFinderWalkDirEntry) -> bool {
        if self
            .skipped_dirs
            .iter()
            .any(|skip_pattern| skip_pattern.matches_path(dir_entry.file_name().as_ref()))
        {
            return false;
        }
        let path = dir_entry.path();
        let slash_path = path.to_slash_lossy();
        !self
            .skipped_dirs
            .iter()
            .any(|skip_pattern| skip_pattern.matches(slash_path.as_ref()))
    }
}

fn is_js_ts_file(s: &OsStr) -> bool {
    for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
        if s.as_encoded_bytes().ends_with(ext.as_bytes()) {
            return true;
        }
    }
    false
}
