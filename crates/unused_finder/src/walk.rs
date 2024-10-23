use crate::ignore_file::IgnoreFile;
use crate::logger::Logger;
use crate::parse::exports_visitor_runner::SourceFileParseError;
use crate::parse::{get_file_import_export_info, RawImportExportInfo};
use crate::walked_file::{WalkedPackage, WalkedSourceFile};
use ahashmap::AHashMap;
use anyhow::Context;
use ignore::overrides::OverrideBuilder;
use ignore::DirEntry;
use rayon::iter::Either;
use rayon::prelude::*;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
enum WalkedFile {
    SourceFile(PathBuf, RawImportExportInfo),
    PackageJson(WalkedPackage),
    IgnoreFile(IgnoreFile),
}

#[derive(Debug)]
pub struct RepoPackages {
    pub packages: Vec<WalkedPackage>,
    pub packages_by_name: AHashMap<String, usize>,
    pub packages_by_path: AHashMap<PathBuf, usize>,
}

impl RepoPackages {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            packages_by_name: AHashMap::default(),
            packages_by_path: AHashMap::default(),
        }
    }

    pub fn add(&mut self, package: WalkedPackage) -> Result<(), anyhow::Error> {
        let package_name = match package.package_json.name {
            Some(ref name) => name,
            None => {
                return Err(anyhow::anyhow!(
                    "Package at path {:?} has no name",
                    package.package_path
                ))
            }
        };
        let id = self.packages.len();
        let name_entry = match self.packages_by_name.entry(package_name.clone()) {
            // Err if it already exists
            ahashmap::hash_map::Entry::Occupied(_) => {
                return Err(anyhow::anyhow!(
                    "Package with name {} already exists",
                    package_name
                ))
            }
            ahashmap::hash_map::Entry::Vacant(entry) => entry,
        };
        name_entry.insert(id);

        self.packages_by_path
            .insert(package.package_path.clone(), id);

        self.packages.push(package);
        Ok(())
    }

    // look up a package by its package name
    pub fn get_by_name(&self, name: &str) -> Option<&WalkedPackage> {
        self.packages_by_name
            .get(name)
            .map(|&id| &self.packages[id])
    }

    // look up a package by its exact path, including "package.json"
    #[allow(dead_code)]
    pub fn get_by_path(&self, path: impl AsRef<Path>) -> Option<&WalkedPackage> {
        self.packages_by_path
            .get(path.as_ref())
            .map(|&id| &self.packages[id])
    }

    // Look up a package by a path that is a child of the package's path
    pub fn get_by_child_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Option<&WalkedPackage>, anyhow::Error> {
        let path = path.as_ref();
        let mut current_path = path;
        const MAX_ITER: usize = 1000;
        for _ in 0..MAX_ITER {
            if let Some(id) = self
                .packages_by_path
                .get(&current_path.join("package.json"))
            {
                return Ok(Some(&self.packages[*id]));
            }

            current_path = match current_path.parent() {
                Some(p) => p,
                None => return Ok(None),
            };
        }

        Err(anyhow::anyhow!(
            "Exceeded max iterations while looking up package for file at path {:?}",
            path
        ))
    }

    #[allow(dead_code)]
    pub fn contains_name(&self, name: &str) -> bool {
        self.packages_by_name.contains_key(name)
    }

    pub fn contains_path(&self, path: impl AsRef<Path>) -> bool {
        self.packages_by_path.contains_key(path.as_ref())
    }

    pub fn iter_names(&self) -> impl Iterator<Item = &String> {
        self.packages_by_name.keys()
    }
}

#[derive(Debug)]
pub struct WalkedFiles {
    // hashmap of walked packages,
    // fomatted by package name -> WalkedPackage
    pub packages: RepoPackages,
    // List of walked source files
    pub source_files: Vec<WalkedSourceFile>,
    // List of files to ignore unused symbols in entirely
    pub ignore_files: Vec<IgnoreFile>,
}

/// Walks the root paths of a project and returns a list of source files and packages
pub fn walk_src_files(
    logger: impl Logger,
    root_paths: &[impl AsRef<Path> + Debug],
    repo_root_path: impl AsRef<Path>,
    ingnored_filenames: &[impl AsRef<str> + Debug],
) -> Result<WalkedFiles, anyhow::Error> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<WalkedFile, anyhow::Error>>();
    let mut all_walked_files: Vec<WalkedFile> = Vec::new();
    // create a mutable reference to all_walked_files, so we can
    // pass ownership of it to the collector thread
    let all_walked_ref = &mut all_walked_files;
    // creates a new scope for spawning threads. This allows us to
    // safely borrow &Logger, because the scope guarantees the threads
    // will be joined after the scope ends.
    std::thread::scope(|scope| -> Result<(), anyhow::Error> {
        let collector_thread = scope.spawn(move || {
            for file in rx {
                match file {
                    Ok(file) => {
                        all_walked_ref.push(file);
                    }
                    Err(e) => {
                        logger.log(format!("Error during walk: {:?}", e));
                    }
                }
            }
        });

        // Parallel walk of each root path in sequence
        for root_path in root_paths {
            match build_walk(root_path, ingnored_filenames) {
                Ok(walk) => collect_walk(walk, &tx),
                Err(e) => {
                    return Err(anyhow!(format!(
                        "Error constructing walk over {}: {}",
                        root_path.as_ref().display(),
                        e
                    )));
                }
            }
        }

        let ignore_file =
            IgnoreFile::read(repo_root_path.as_ref().to_path_buf().join(".unusedignore"))?;
        tx.send(Ok(WalkedFile::IgnoreFile(ignore_file))).unwrap();

        // drop the sender to signal the collector thread to stop
        drop(tx);

        collector_thread.join().unwrap();

        Ok(())
    })?;

    // after the walk is complete, collec the results. This allows us to
    // defer resolving the package ownership of source files until all
    // source files and packages have been collected.
    let (result, pkg_assignment_errs) = collect_results(all_walked_files.into_iter());
    for error in pkg_assignment_errs {
        logger.log(format!("Error during package assignment: {:?}", error));
    }

    Ok(result)
}

/// The default set of patterns to skip during the walk
///
/// This can be overridden by the user by specifying individual negations to
/// these patterns in the `skip` field of the UnusedFinderConfig.
pub const DEFAULT_OVERRIDE_PATTERNS: &[&str] = &["!node_modules", "!lib", "!target"];

fn build_walk(
    root_path: impl AsRef<Path>,
    ingnored_filenames: &[impl AsRef<str>],
) -> Result<ignore::WalkParallel, anyhow::Error> {
    // Build overrides matcher
    let mut override_builder = OverrideBuilder::new(root_path.as_ref());
    // permit all matches by default
    override_builder
        .add("*")
        .expect("default glob should be valid");
    for dir in DEFAULT_OVERRIDE_PATTERNS {
        override_builder
            .add(dir)
            .expect("builtin overrides should be valid");
    }

    // add user-specified ignored filenames
    for filename in ingnored_filenames {
        let as_ref = filename.as_ref();
        let inverted_ignore = if as_ref.starts_with("!") {
            as_ref[2..].to_string()
        } else {
            format!("!{}", as_ref)
        };
        // note: Overrides use an inverted ignore pattern -- globs must be prefixed with
        // `!` to be treated as an ignore pattern
        override_builder
            .add(&inverted_ignore)
            .with_context(|| format!("Failed to override {:?}", as_ref))?;
    }

    // add overrides to the builder
    let overrides = override_builder
        .build()
        .context("Failed to build overrides")?;

    // build the walker
    let mut walk_builder = ignore::WalkBuilder::new(root_path);

    // configure the builder;
    walk_builder.standard_filters(false).hidden(true);
    if !overrides.is_empty() {
        walk_builder.overrides(overrides);
    }

    Ok(walk_builder.build_parallel())
}

fn collect_walk(
    walk: ignore::WalkParallel,
    tx: &std::sync::mpsc::Sender<Result<WalkedFile, anyhow::Error>>,
) {
    walk.run(|| {
        Box::new(move |result| -> ignore::WalkState {
            match result {
                Ok(entry) => {
                    let walked_file = visit_entry(entry);
                    let send_result = match walked_file {
                        Ok(Some(file)) => tx.send(Ok(file)),
                        Ok(None) => return ignore::WalkState::Continue,
                        Err(e) => tx.send(Err(e)),
                    };

                    send_result.unwrap();
                }
                Err(e) => {
                    tx.send(Err(anyhow!(e))).unwrap();
                }
            };
            ignore::WalkState::Continue
        })
    });
}

fn collect_results(
    walked_files: impl Iterator<Item = WalkedFile>,
) -> (WalkedFiles, Vec<anyhow::Error>) {
    // partition the results
    let mut packages = RepoPackages::new();
    let mut source_files: Vec<(PathBuf, RawImportExportInfo)> = Vec::new();
    let mut ignore_files: Vec<IgnoreFile> = Vec::new();
    let mut errors: Vec<anyhow::Error> = Vec::new();
    for file in walked_files.into_iter() {
        match file {
            // noop the source files
            WalkedFile::SourceFile(file_path, imports) => {
                source_files.push((file_path, imports));
            }
            WalkedFile::PackageJson(file) => match packages.add(file) {
                Ok(_) => {}
                Err(e) => errors.push(e),
            },
            WalkedFile::IgnoreFile(file) => ignore_files.push(file),
        }
    }

    // Use `packages` to add owners to our list of source files
    let (source_files, mut pkg_assignment_errs): (Vec<WalkedSourceFile>, Vec<anyhow::Error>) = source_files
        .into_par_iter()
        .map(
            |(source_file_path, import_export_info)| -> Result<WalkedSourceFile, anyhow::Error> {
                Ok(WalkedSourceFile {
                    owning_package: packages
                        .get_by_child_path(&source_file_path)?
                        .and_then(|package| package.package_json.name.clone()),
                    source_file_path,
                    import_export_info,
                })
            },
        )
        .partition_map(split_errs);

    let result = WalkedFiles {
        packages,
        source_files,
        ignore_files,
    };

    errors.append(&mut pkg_assignment_errs);
    (result, errors)
}

// callback meant to be called during a file walk of a directory
// (e.g. with jwalk's process_read_dir() callback)
fn visit_entry(entry: DirEntry) -> Result<Option<WalkedFile>, anyhow::Error> {
    let dir_path = entry.path();
    let file_name = entry.file_name();
    if equals_os_str(file_name, "package.json") {
        WalkedPackage::from_path(dir_path)
            .with_context(|| "Failed to walk package.json")
            .map(|package| Some(WalkedFile::PackageJson(package)))
    } else if equals_os_str(file_name, ".unusedignore") {
        let ignore_file = IgnoreFile::read(dir_path.to_path_buf())?;
        Ok(Some(WalkedFile::IgnoreFile(ignore_file)))
    } else if is_js_ts_file(file_name) {
        // Source file [.ts, .tsx, .js, .jsx]
        match get_file_import_export_info(entry.path()).map(|import_export_info| {
            Some(WalkedFile::SourceFile(
                dir_path.to_path_buf(),
                import_export_info,
            ))
        }) {
            // Pass-through results
            Ok(file) => Ok(file),
            // Skip auto-generated files -- they are not relevant to analysis
            Err(SourceFileParseError::AutogeneratedFile) => Ok(None),
            // Return other parse errors as anyhow errors
            Err(e) => Err(e).with_context(|| format!("Failed to read source file: {:?}", dir_path)),
        }
    } else {
        Ok(None)
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

fn equals_os_str(s: &OsStr, t: &str) -> bool {
    s.as_bytes().eq(t.as_bytes())
}

fn split_errs<A, B>(x: Result<A, B>) -> Either<A, B> {
    match x {
        Ok(file) => Either::Left(file),
        Err(e) => Either::Right(e),
    }
}

#[cfg(test)]
mod test {
    use crate::logger::StdioLogger;

    use super::*;
    use test_tmpdir::test_tmpdir;

    #[test]
    fn test_discovers_root_unusedignore() {
        let tmpdir = test_tmpdir!(
            ".unusedignore" => r#"
                src/ignored.js
            "#,
            "packages/.unusedignore" => r#"
                *.pkgs-ignored.ts
            "#,
            "shared/.unusedignore" => r#"
                *.shared-ignored.ts
            "#
        );

        let test_logger = StdioLogger {};
        let walk_result = walk_src_files(
            &test_logger,
            &[tmpdir.root().join("packages"), tmpdir.root().join("shared")],
            tmpdir.root(),
            &["*.ignored.ts"],
        );

        let walk_result = walk_result.unwrap();
        assert!(walk_result
            .ignore_files
            .iter()
            .any(|x| x.path == tmpdir.root().join("packages")),);
        assert!(walk_result
            .ignore_files
            .iter()
            .any(|x| x.path == tmpdir.root().join("shared")),);
        assert!(walk_result
            .ignore_files
            .iter()
            .any(|x| x.path == tmpdir.root()));
        assert_eq!(walk_result.ignore_files.len(), 3);
    }
}
