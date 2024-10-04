use crate::parse::RawImportExportInfo;
use crate::walked_file::WalkedSourceFile;
use anyhow::Result;
use rayon::prelude::*;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};
use swc_core::{common::source_map::SmallPos, ecma::loader::resolve::Resolve};

// pub fn find_unused_items(
//     config: FindUnusedItemsConfig,
// ) -> Result<UnusedFinderReport, js_err::JsErr> {
// }

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
//                     ],i
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
