use crate::logger::Logger;
use crate::parse::FileImportExportInfo;
use crate::utils::{
    jwalk_src_subtree, process_async_imported_paths, process_executed_paths, process_exports_from,
    process_import_path_ids, process_require_paths,
};
use crate::walked_file::UnusedFinderSourceFile;
use crate::walked_file::WalkedFile;
use anyhow::Result;
use packagejson::PackageJson;
use rayon::iter::Either;
use rayon::prelude::*;
use serde::Deserialize;
use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};
use swc_core::{common::source_map::SmallPos, ecma::loader::resolve::Resolve};

#[cfg_attr(feature = "napi", napi(object))]
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindUnusedItemsConfig {
    // Trace exported symbols that are not imported anywhere in the project
    #[serde(default)]
    pub report_exported_items: bool,
    // Root paths to walk as source files
    #[serde(alias = "pathsToRead")]
    pub root_paths: Vec<String>,
    // Path to the root tsconfig.paths.json file used to resolve ts imports between projects.
    // Note: this should be removed and replaced with normal node resolution.
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

// Represents a single exported item in a file
#[derive(Debug, Clone)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct ExportedItemReport {
    pub id: String,
    pub start: i32,
    pub end: i32,
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct UnusedFinderReport {
    // files that are completely unused
    pub unused_files: Vec<String>,
    // items that are unused within files
    pub unused_files_items: HashMap<String, Vec<ExportedItemReport>>,
}

impl Display for UnusedFinderReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut unused_files = self
            .unused_files
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        unused_files.sort();
        let unused_files_set = self
            .unused_files
            .iter()
            .map(|x| x.to_string())
            .collect::<HashSet<String>>();

        for file_path in unused_files.iter() {
            match self.unused_files_items.get(file_path) {
                Some(items) => writeln!(
                    f,
                    "{} is completely unused ({} item{})",
                    file_path,
                    items.len(),
                    if items.len() > 1 { "s" } else { "" },
                )?,
                None => writeln!(f, "{} is completely unused", file_path)?,
            };
        }

        for (file_path, items) in self.unused_files_items.iter() {
            if unused_files_set.contains(file_path) {
                continue;
            }
            writeln!(
                f,
                "{} is partially unused ({} unused export{}):",
                file_path,
                items.len(),
                if items.len() > 1 { "s" } else { "" },
            )?;
            for item in items.iter() {
                writeln!(f, "  - {}", item.id)?;
            }
        }

        Ok(())
    }
}

pub fn create_report_map_from_flattened_files(
    flattened_walk_file_data: &Vec<UnusedFinderSourceFile>,
) -> HashMap<String, Vec<ExportedItemReport>> {
    let file_path_exported_items_map: HashMap<String, Vec<ExportedItemReport>> =
        flattened_walk_file_data
            .par_iter()
            .map(|file| {
                let ids = file
                    .import_export_info
                    .exported_ids
                    .iter()
                    .map(|exported_item| ExportedItemReport {
                        id: exported_item.metadata.export_kind.to_string(),
                        start: exported_item.metadata.span.lo.to_usize() as i32,
                        end: exported_item.metadata.span.hi.to_usize() as i32,
                    })
                    .collect();
                (file.source_file_path.clone(), ids)
            })
            .collect();
    file_path_exported_items_map
}

#[derive(Debug)]
pub struct WalkFileResult {
    // Map of package name to (path, package.json)
    pub packages: HashMap<String, (String, PackageJson)>,
    // Map of source file path to source file data
    pub source_files: HashMap<String, UnusedFinderSourceFile>,
}

pub fn walk_src_files(
    logger: impl Logger,
    root_paths: &Vec<String>,
    skipped_dirs: &Arc<Vec<glob::Pattern>>,
    skipped_items: &Arc<Vec<regex::Regex>>,
) -> WalkFileResult {
    let (mut source_files, packages): (Vec<UnusedFinderSourceFile>, Vec<(String, PackageJson)>) =
        root_paths
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
                |file: WalkedFile| -> Either<UnusedFinderSourceFile, (String, PackageJson)> {
                    match file {
                        WalkedFile::SourceFile(file) => Either::Left(file),
                        WalkedFile::PackageJson(path, packagejson) => {
                            Either::Right((path, packagejson))
                        }
                    }
                },
            );

    let packages_map: HashMap<String, (String, PackageJson)> = packages
        .into_iter()
        .map(|(path, package_json)| {
            (
                package_json
                    .name
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or_else(|| "")
                    .to_string(),
                (path, package_json),
            )
        })
        .collect();

    WalkFileResult {
        packages: packages_map,
        source_files: source_files
            .drain(0..)
            .map(|f| (f.source_file_path.clone(), f))
            .collect(),
    }
}

// pub fn find_unused_items(
//     config: FindUnusedItemsConfig,
// ) -> Result<UnusedFinderReport, js_err::JsErr> {
// }

// Looks in cwd for a file called `.unusedignore`
// allowed items can be:
// - specific file paths like `shared/internal/owa-react-hooks/src/useWhyDidYouUpdate.ts`
// - glob patterns (similar to a `.gitignore` file) `shared/internal/owa-datetime-formatters/**`
pub fn read_allow_list() -> Result<Vec<glob::Pattern>> {
    return match std::fs::read_to_string(".unusedignore") {
        Ok(list) => list
            .split("\n")
            .enumerate()
            .map(|(idx, line)| {
                glob::Pattern::new(line)
                    .map_err(|e| anyhow!("line {}: failed to parse pattern: {}", idx, e))
            })
            .collect::<Result<Vec<glob::Pattern>, anyhow::Error>>(),
        Err(e) => Err(anyhow!("failed to read .unusedignore file: {}", e)),
    };
}

pub fn process_import_export_info(
    f: &mut FileImportExportInfo,
    source_file_path: &str,
    resolver: &dyn Resolve,
) -> Result<()> {
    process_executed_paths(f, source_file_path, resolver)?;
    process_async_imported_paths(f, source_file_path, resolver)?;
    process_exports_from(f, source_file_path, resolver)?;
    process_require_paths(f, source_file_path, resolver)?;
    process_import_path_ids(f, source_file_path, resolver)?;

    Ok(())
}

// #[cfg(test)]
// mod test {
//     use crate::{ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport};

//     use super::find_unused_items;

//     #[test]
//     fn test_format_report() {
//         let report = UnusedFinderReport {
//             unused_files: vec!["file1".to_string()],
//             unused_files_items: vec![
//                 (
//                     "file1".to_string(),
//                     vec![ExportedItemReport {
//                         id: "unused".to_string(),
//                         start: 1,
//                         end: 2,
//                     }],
//                 ),
//                 (
//                     "file2".to_string(),
//                     vec![
//                         ExportedItemReport {
//                             id: "item1".to_string(),
//                             start: 1,
//                             end: 2,
//                         },
//                         ExportedItemReport {
//                             id: "item2".to_string(),
//                             start: 3,
//                             end: 4,
//                         },
//                     ],
//                 ),
//             ]
//             .into_iter()
//             .collect(),
//         };

//         assert_eq!(
//             format!("{}", report),
//             r#"file1 is completely unused (1 item)
// file2 is partially unused (2 unused exports):
//   - item1
//   - item2
// "#
//         );
//     }

//     #[test]
//     fn test_error_in_glob() {
//         let result = find_unused_items(FindUnusedItemsConfig {
//             root_paths: vec!["tests/unused_finder".to_string()],
//             ts_config_path: "tests/unused_finder/tsconfig.json".to_string(),
//             skipped_dirs: vec![".....///invalidpath****".to_string()],
//             skipped_items: vec!["[A-Z].*".to_string(), "something".to_string()],
//             ..Default::default()
//         });
//         assert!(result.is_err());
//         assert_eq!(
//             result.unwrap_err().message(),
//             "Pattern syntax error near position 21: wildcards are either regular `*` or recursive `**`"
//         )
//     }

//     #[test]
//     fn test_error_in_regex() {
//         let result = find_unused_items(FindUnusedItemsConfig {
//             root_paths: vec!["tests/unused_finder".to_string()],
//             ts_config_path: "tests/unused_finder/tsconfig.json".to_string(),
//             skipped_items: vec!["[A-Z.*".to_string(), "something".to_string()],
//             ..Default::default()
//         });
//         assert!(result.is_err());
//         assert_eq!(
//             result.unwrap_err().message(),
//             "regex parse error:\n    [A-Z.*\n    ^\nerror: unclosed character class"
//         )
//     }
// }
