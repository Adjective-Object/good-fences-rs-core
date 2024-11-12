use core::{
    convert::Into,
    option::Option::{None, Some},
};
use std::fmt::Display;

use ahashmap::AHashMap;
use debug_print::debug_println;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use swc_common::source_map::SmallPos;

use crate::{
    graph::{Graph, GraphFile},
    parse::ExportedSymbol,
    tag::UsedTag,
    UnusedFinderResult, UsedTagEnum,
};

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SymbolReportWithTags {
    pub symbol: SymbolReport,
    pub tags: Vec<UsedTagEnum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileInfo {
    tags: Vec<UsedTagEnum>,
    symbols: AHashMap<String, Vec<SymbolReport>>,
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnusedFinderReport {
    /// Files that are completely unused
    pub unused_files: Vec<String>,
    /// Exported symbols that are unused within files
    /// note that this intentionally uses a std HashMap type to guarantee napi
    /// compatibility
    pub unused_symbols: AHashMap<String, Vec<SymbolReport>>,

    /// File tag information for files that are used.
    pub extra_file_tags: AHashMap<String, Vec<UsedTagEnum>>,
    pub extra_symbol_tags: AHashMap<String, Vec<SymbolReportWithTags>>,
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

fn extract_symbols<T: Send + Sync>(
    graph: &Graph,
    include_symbol: impl Fn(&GraphFile, &ExportedSymbol) -> Option<T> + Sync,
) -> AHashMap<String, Vec<T>> {
    graph
        .files
        .par_iter()
        .filter_map(|graph_file| -> Option<(String, Vec<T>)> {
            // Find all used symbols in the file
            let unused_symbols = graph_file
                .import_export_info
                .iter_exported_symbols()
                .filter_map(|(_, symbol): (_, &ExportedSymbol)| -> Option<T> {
                    include_symbol(graph_file, symbol)
                })
                .collect::<Vec<_>>();

            if unused_symbols.is_empty() {
                return None;
            }

            Some((
                graph_file.file_path.to_string_lossy().to_string(),
                unused_symbols,
            ))
        })
        .collect::<AHashMap<String, Vec<T>>>()
}

fn is_used(tags: &UsedTag) -> bool {
    tags.contains(UsedTag::FROM_ENTRY)
        || tags.contains(UsedTag::FROM_IGNORED)
        || tags.contains(UsedTag::FROM_TEST)
        || tags.contains(UsedTag::TYPE_ONLY)
}
fn include_extra(tags: &UsedTag) -> bool {
    !tags.is_empty() && *tags != UsedTag::FROM_ENTRY
}

impl From<&UnusedFinderResult> for UnusedFinderReport {
    fn from(value: &UnusedFinderResult) -> Self {
        let mut unused_files: Vec<String> = value
            .graph
            .files
            .par_iter()
            .filter_map(|file| {
                if is_used(&file.file_tags) {
                    return None;
                }
                Some(file.file_path.to_string_lossy().to_string())
            })
            .collect();
        unused_files.sort();
        let extra_file_tags = value
            .graph
            .files
            .par_iter()
            .filter_map(|file| {
                if !include_extra(&file.file_tags) {
                    None
                } else {
                    Some((
                        file.file_path.to_string_lossy().to_string(),
                        file.file_tags.into(),
                    ))
                }
            })
            .collect();

        let unused_symbols =
            extract_symbols(&value.graph, |file, symbol_name| -> Option<SymbolReport> {
                let default: UsedTag = Default::default();
                let symbol_bitflags: &UsedTag =
                    file.symbol_tags.get(symbol_name).unwrap_or(&default);

                if is_used(symbol_bitflags) {
                    // don't return used symbols
                    return None;
                }

                let ast_symbol = file.import_export_info.exported_ids.get(symbol_name)?;

                Some(SymbolReport {
                    id: symbol_name.to_string(),
                    start: ast_symbol.span.lo().to_u32(),
                    end: ast_symbol.span.hi().to_u32(),
                })
            });

        let extra_symbol_tags = extract_symbols(
            &value.graph,
            |file, symbol_name| -> Option<SymbolReportWithTags> {
                let default: UsedTag = Default::default();
                let symbol_bitflags: &UsedTag =
                    file.symbol_tags.get(symbol_name).unwrap_or(&default);
                debug_println!(
                    "visit symbol {}:{}  ({})",
                    file.file_path.display(),
                    symbol_name,
                    symbol_bitflags
                );

                if !include_extra(symbol_bitflags) {
                    // don't return symbols that are used or symbols that are truly unused
                    return None;
                }

                let ast_symbol = file.import_export_info.exported_ids.get(symbol_name)?;

                Some(SymbolReportWithTags {
                    symbol: SymbolReport {
                        id: symbol_name.to_string(),
                        start: ast_symbol.span.lo().to_u32(),
                        end: ast_symbol.span.hi().to_u32(),
                    },
                    tags: (*symbol_bitflags).into(),
                })
            },
        );

        UnusedFinderReport {
            unused_files,
            unused_symbols,
            // TODO collect tags from symbols are "used", but not
            // entrypoints into the project
            extra_file_tags,
            extra_symbol_tags,
        }
    }
}
