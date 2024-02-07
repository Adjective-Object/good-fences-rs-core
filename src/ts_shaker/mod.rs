

// Retrieve all files in specified directories.
use anyhow::Result;
use path_slash::PathBufExt;
pub mod syntax_scanner;
use crate::unused_finder::graph::GraphFile;
// pub fn retrieve_files_import_export_info(dirs_to_scan: Vec<String>) -> Result<Vec<GraphFile>> {
//     let mut files = vec![];
//     for dir in dirs_to_scan {
//         let mut dir_files = std::fs::read_dir(dir)?
//             .filter_map(|entry| entry.ok())
//             .filter(|entry| entry.path().is_file())
//             .filter(|entry| !entry.path().to_slash().unwrap().to_string().contains("/node_modules/"))
//             .filter(|entry| !entry.path().to_slash().unwrap().to_string().contains("/lib/"))
//             .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "ts" || ext == "tsx"))
//             .map(|entry| entry.path())
//             .map(|path| {
//                 let file_path = path.to_slash().unwrap().to_string();
//             })
//             .collect::<Vec<_>>();
//         files.append(&mut dir_files);
//     }
//     Ok(files)
// }
// Check for the initial frontiner/entry points.
// For each file, parse the file and extract the imports and exports.
// Mark the entry points as used.
// Create a graph of the files and their imports and exports.
// Create a queue of the entry points.
// Create a set of visited files.
// Create a set of unused files.
// Create a set of unused exports.
// Create a set of recently visited files that will serve as the new frontier
// For each import, mark the imported file as used.
// For each item imported, mark the item as used within the file.
// do loop
// - if file was not marked as used, mark the file as unused.
// - If the item was imported from a `export * from '...'` mark the item as used in the exporting file.
// while the item was imported from a `export * from '...'`;
