use std::{collections::HashMap, fmt::Display};

use rayon::prelude::*;
use swc_core::common::source_map::SmallPos;

use crate::UnusedFinderResult;

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct UnusedSymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "napi", napi(object))]
pub struct UnusedFinderReport {
    // files that are completely unused
    pub unused_files: Vec<String>,
    // items that are unused within files
    // note that this intentionally uses the std HashMap type to guarantee napi
    // compatibility
    pub unused_symbols: HashMap<String, Vec<UnusedSymbolReport>>,
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
            .collect::<Vec<String>>();

        for file_path in unused_files.iter() {
            match self.unused_symbols.get(file_path) {
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

        for (file_path, items) in self.unused_symbols.iter() {
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

impl From<&UnusedFinderResult> for UnusedFinderReport {
    fn from(value: &UnusedFinderResult) -> Self {
        let mut unused_files: Vec<String> = value
            .graph
            .files
            .par_iter()
            .filter_map(|file| {
                if file.is_file_used {
                    None
                } else {
                    Some(file.file_path.to_string_lossy().to_string())
                }
            })
            .collect();
        unused_files.sort();

        let unused_symbols: HashMap<String, Vec<UnusedSymbolReport>> = value.graph
        .files
        .par_iter()
        .filter_map(|graph_file| -> Option<(String, Vec<UnusedSymbolReport>)> {
            if graph_file.unused_named_exports.is_empty() {
                return None;
            }

            let file_unused_symbols = graph_file.unused_named_exports.iter().filter_map(
                |symbol| -> Option<UnusedSymbolReport> {
                    let import_export_info = graph_file
                        .import_export_info.exported_ids
                        .get(symbol)
                        .expect("IDs from a graph file must also be contained in the graphfile's ImportExportInfo");
                    if import_export_info.allow_unused {
                        return None
                    }
                    Some(UnusedSymbolReport{
                        id: symbol.to_string(),
                        start: import_export_info.span.lo().to_u32(),
                        end: import_export_info.span.hi().to_u32(),
                    })
                }
            ).collect::<Vec<_>>();

            Some((graph_file.file_path.to_string_lossy().to_string(), file_unused_symbols))
        })
        .collect::<HashMap<String, Vec<UnusedSymbolReport>>>();

        UnusedFinderReport {
            unused_files,
            unused_symbols,
        }
    }
}
