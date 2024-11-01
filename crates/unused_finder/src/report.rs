use core::{
    convert::Into,
    option::Option::{None, Some},
};
use std::fmt::Display;

use ahashmap::AHashMap;
use itertools::Either;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use swc_common::source_map::SmallPos;

use crate::{
    graph::{Graph, GraphFile, UsedTag},
    parse::ExportedSymbol,
    tag::UsedTagEnum,
    UnusedFinderResult,
};

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

<<<<<<< HEAD
#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedTagEnum {
    Entry,
    Ignored,
    TypeOnly,
||||||| parent of 1b97a00 (unused_finder: track test files, return tagged symbols)
#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedTagEnum {
    Entry,
    Ignored,
=======
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SymbolReportWithTags {
    pub symbol: SymbolReport,
    pub tags: Vec<UsedTagEnum>,
>>>>>>> 1b97a00 (unused_finder: track test files, return tagged symbols)
}

<<<<<<< HEAD
        let mut result = Vec::new();
        if flags.contains(UsedTag::FROM_ENTRY) {
            result.push(UsedTagEnum::Entry);
        }
        if flags.contains(UsedTag::FROM_IGNORED) {
            result.push(UsedTagEnum::Ignored);
        }
        if flags.contains(UsedTag::TYPE_ONLY) {
            result.push(UsedTagEnum::TypeOnly);
        }

        Some(result)
    }
||||||| parent of 1b97a00 (unused_finder: track test files, return tagged symbols)
        let mut result = Vec::new();
        if flags.contains(UsedTag::FROM_ENTRY) {
            result.push(UsedTagEnum::Entry);
        }
        if flags.contains(UsedTag::FROM_IGNORED) {
            result.push(UsedTagEnum::Ignored);
        }

        Some(result)
    }
=======
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileInfo {
    tags: Vec<UsedTagEnum>,
    symbols: AHashMap<String, Vec<SymbolReport>>,
>>>>>>> 1b97a00 (unused_finder: track test files, return tagged symbols)
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

impl From<&UnusedFinderResult> for UnusedFinderReport {
    fn from(value: &UnusedFinderResult) -> Self {
        let (mut unused_files, extra_file_tags): (Vec<String>, AHashMap<String, Vec<UsedTagEnum>>) =
            value
                .graph
                .files
                .par_iter()
                .filter(|file| {
                    !file.file_tags.contains(UsedTag::FROM_ENTRY)
                        && !file.file_tags.contains(UsedTag::FROM_TEST)
                        && !file.file_tags.contains(UsedTag::FROM_IGNORED)
                })
                .partition_map(|file| {
                    if !file.file_tags.contains(UsedTag::FROM_ENTRY) {
                        Either::Left(file.file_path.to_string_lossy().to_string())
                    } else {
                        Either::Right((
                            file.file_path.to_string_lossy().to_string(),
                            file.file_tags.into(),
                        ))
                    }
                });
        unused_files.sort();

        let unused_symbols =
            extract_symbols(&value.graph, |file, symbol_name| -> Option<SymbolReport> {
                let default: UsedTag = Default::default();
                let symbol_bitflags: &UsedTag =
                    file.symbol_tags.get(symbol_name).unwrap_or(&default);
                println!(
                    "visit symbol {}:{}  ({})",
                    file.file_path.display(),
                    symbol_name,
                    symbol_bitflags
                );

                if symbol_bitflags.contains(UsedTag::FROM_ENTRY)
                    || symbol_bitflags.contains(UsedTag::FROM_TEST)
                    || symbol_bitflags.contains(UsedTag::FROM_IGNORED)
                {
                    // don't return used symbols
                    return None;
                }

                let ast_symbol = file.import_export_info.exported_ids.get(symbol_name)?;

<<<<<<< HEAD
                            if symbol_bitflags.contains(UsedTag::FROM_ENTRY)
                                || symbol_bitflags.contains(UsedTag::FROM_TEST)
                                || symbol_bitflags.contains(UsedTag::FROM_IGNORED)
                                || symbol_bitflags.contains(UsedTag::TYPE_ONLY)
                            {
                                // don't return used symbols
                                return None;
                            }
||||||| parent of 1b97a00 (unused_finder: track test files, return tagged symbols)
                            if symbol_bitflags.contains(UsedTag::FROM_ENTRY)
                                || symbol_bitflags.contains(UsedTag::FROM_TEST)
                                || symbol_bitflags.contains(UsedTag::FROM_IGNORED)
                            {
                                // don't return used symbols
                                return None;
                            }
=======
                Some(SymbolReport {
                    id: symbol_name.to_string(),
                    start: ast_symbol.span.lo().to_u32(),
                    end: ast_symbol.span.hi().to_u32(),
                })
            });
>>>>>>> 1b97a00 (unused_finder: track test files, return tagged symbols)

        let extra_symbol_tags = extract_symbols(
            &value.graph,
            |file, symbol_name| -> Option<SymbolReportWithTags> {
                let default: UsedTag = Default::default();
                let symbol_bitflags: &UsedTag =
                    file.symbol_tags.get(symbol_name).unwrap_or(&default);
                println!(
                    "visit symbol {}:{}  ({})",
                    file.file_path.display(),
                    symbol_name,
                    symbol_bitflags
                );

                if symbol_bitflags.contains(UsedTag::FROM_ENTRY)
                    || !symbol_bitflags.contains(UsedTag::FROM_TEST)
                        && !symbol_bitflags.contains(UsedTag::FROM_IGNORED)
                {
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
