// extern crate find_ts_imports;
// extern crate serde;
// use crate::fence::parse_fence_file;
// use find_ts_imports::{parse_source_file_imports, SourceFileImportData};
// use jwalk::WalkDirGeneric;
// use serde::Deserialize;
// use std::collections::HashSet;

// fn should_retain_file(s: &str) -> bool {
//   s == "fence.json" || s.ends_with(".ts") || s.ends_with(".tsx")
// }

// #[derive(Debug, Deserialize)]
// pub struct Fence {
//   fence_path: Box<String>,
// }

// #[derive(Debug, Deserialize)]
// pub struct SourceFile {
//   // ref to the strings of tags that apply to this file
//   tags: Vec<String>,
//   imports: SourceFileImportData,
// }

// #[derive(Debug, Deserialize)]
// pub enum WalkFileData {
//   Fence(Fence),
//   SourceFile(SourceFile),
//   Nothing,
// }

// impl Default for WalkFileData {
//   fn default() -> WalkFileData {
//     WalkFileData::Nothing
//   }
// }

// type TagList = Vec<String>;

// pub fn discover_fences_and_files(start_path: &str) -> Vec<WalkFileData> {
//   let walk_dir = WalkDirGeneric::<(TagList, WalkFileData)>::new(start_path).process_read_dir(
//     |read_dir_state, children| {
//       // Custom filter -- retain only directories and fence.json files
//       children.retain(|dir_entry_result| {
//         dir_entry_result
//           .as_ref()
//           .map(|dir_entry| {
//             dir_entry.file_type.is_dir()
//               || match dir_entry.file_name.to_str() {
//                 Some(file_name_str) => should_retain_file(file_name_str),
//                 None => false,
//               }
//           })
//           .unwrap_or(false)
//       });

//       // Look for fence.json files and add them to the tag list
//       for child_result in children {
//         match child_result {
//           Ok(dir_entry) => {
//             let f = dir_entry.file_name.to_str();
//             match f {
//               Some(file_path) => {
//                 if file_path.ends_with("fence.json") {
//                   // TODO process fence, add to the parsing context
//                 }
//               }
//               Nothing => panic!("c_str was not a string?"),
//             }
//           }
//           // TODO maybe don't swallow errors here? not sure
//           // when this error even fires.
//           Err(_) => (),
//         }
//       }

//       for child_result in children {
//         match child_result {
//           Ok(dir_entry) => {
//             let f = dir_entry.file_name.to_str();
//             match f {
//               Some(file_path) => {
//                 if file_path.ends_with(".ts") || file_path.ends_with(".tsx") {
//                   // TODO pull tags from context and put it in here?
//                   // dir_entry.client_state = WalkFileData::SourceFile(SourceFile {
//                   //   imports: parse_source_file_imports(&dir_entry.parent_path.join(file_path)),
//                   //   tags: read_dir_state.clone(),
//                   // });
//                 }
//               }
//               Nothing => panic!("c_str was not a string?"),
//             }
//           }
//           // TODO maybe don't swallow errors here? not sure
//           // when this error even fires.
//           Err(_) => (),
//         }
//       }
//     },
//   );

//   return walk_dir
//     .into_iter()
//     .filter(|e| e.is_ok())
//     .map(|ok| ok.unwrap().client_state)
//     .collect();
// }
