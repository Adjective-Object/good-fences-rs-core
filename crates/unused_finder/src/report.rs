use std::{fmt::Display, path::PathBuf};

use ahashmap::{AHashMap, AHashSet};
use rayon::prelude::*;
use swc_core::common::source_map::SmallPos;

use crate::walked_file::WalkedSourceFile;

// Report of a single exported item in a file
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
    pub unused_files_items: AHashMap<String, Vec<ExportedItemReport>>,
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
            .collect::<AHashSet<String>>();

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

pub fn create_report_map_from_source_files(
    flattened_walk_file_data: &Vec<WalkedSourceFile>,
) -> AHashMap<PathBuf, Vec<ExportedItemReport>> {
    let file_path_exported_items_map: AHashMap<PathBuf, Vec<ExportedItemReport>> =
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
