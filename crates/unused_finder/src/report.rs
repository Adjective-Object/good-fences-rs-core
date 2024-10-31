use std::{collections::BTreeMap, fmt::Display};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use swc_core::common::source_map::SmallPos;

use crate::{graph::UsedTag, parse::ExportedSymbol, UnusedFinderResult};

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
    pub tags: Vec<UsedTagEnum>,
}

#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedTagEnum {
    Entry,
    Ignored,
}
impl From<UsedTag> for Vec<UsedTagEnum> {
    fn from(flags: UsedTag) -> Self {
        let mut result = Vec::new();
        if flags.contains(UsedTag::FROM_ENTRY) {
            result.push(UsedTagEnum::Entry);
        }
        if flags.contains(UsedTag::FROM_IGNORED) {
            result.push(UsedTagEnum::Ignored);
        }
        result
    }
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnusedFinderReport {
    // files that are completely unused
    pub unused_files: Vec<String>,
    // items that are unused within files
    // note that this intentionally uses a std HashMap type to guarantee napi
    // compatibility
    pub unused_symbols: BTreeMap<String, Vec<SymbolReport>>,
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
                if file.file_tags.contains(UsedTag::FROM_ENTRY)
                    || file.file_tags.contains(UsedTag::FROM_TEST)
                    || file.file_tags.contains(UsedTag::FROM_IGNORED)
                {
                    None
                } else {
                    Some(file.file_path.to_string_lossy().to_string())
                }
            })
            .collect();
        unused_files.sort();

        let unused_symbols: BTreeMap<String, Vec<SymbolReport>> = value
            .graph
            .files
            .par_iter()
            .filter_map(|graph_file| -> Option<(String, Vec<SymbolReport>)> {
                // Find all used symbols in the file
                let unused_symbols = graph_file
                    .import_export_info
                    .iter_exported_symbols()
                    .filter_map(
                        |(_, symbol): (_, &ExportedSymbol)| -> Option<SymbolReport> {
                            let symbol_bitflags: UsedTag = graph_file
                                .symbol_tags
                                .get(symbol)
                                .copied()
                                .unwrap_or_default();
                            let ast_symbol =
                                match graph_file.import_export_info.exported_ids.get(symbol) {
                                    Some(ast_symbol) => ast_symbol,
                                    None => {
                                        return None;
                                    }
                                };

                            if symbol_bitflags.contains(UsedTag::FROM_ENTRY)
                                || symbol_bitflags.contains(UsedTag::FROM_TEST)
                                || symbol_bitflags.contains(UsedTag::FROM_IGNORED)
                            {
                                // don't return used symbols
                                return None;
                            }

                            Some(SymbolReport {
                                id: symbol.to_string(),
                                start: ast_symbol.span.lo().to_u32(),
                                end: ast_symbol.span.hi().to_u32(),
                                // for symbols that are not used by entrypoints, return the bitflags where they _are_ used
                                tags: symbol_bitflags.into(),
                            })
                        },
                    )
                    .collect::<Vec<_>>();

                if unused_symbols.is_empty() {
                    return None;
                }

                Some((
                    graph_file.file_path.to_string_lossy().to_string(),
                    unused_symbols,
                ))
            })
            .collect::<BTreeMap<String, Vec<SymbolReport>>>();

        UnusedFinderReport {
            unused_files,
            unused_symbols,
        }
    }
}
