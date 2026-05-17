use core::{
    convert::Into,
    option::Option::{None, Some},
};
use std::fmt::Display;

use ahashmap::AHashMap;
use ast_segmenter::segment_graph::{SegmentGraph, SegmentId, TagSet};
use ast_segmenter::segment_info::Segment;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use swc_common::source_map::SmallPos;

use crate::{
    find_result::{ResultFile, ResultGraph},
    parse::ExportedSymbol,
    tag::UsedTag,
    UnusedFinderConfig, UnusedFinderResult, UsedTagEnum,
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

// Report of a single unused segment (top-level statement) in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
pub struct SegmentReport {
    /// Index of the segment within the file's segment list
    pub segment_idx: usize,
    /// Byte offset of the segment start (1-based, matching SWC convention)
    pub start: u32,
    /// Byte offset of the segment end
    pub end: u32,
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

    /// Unused segments (top-level statements) within files.
    /// Only populated for files that are at least partially used — fully unused
    /// files are already reported in `unused_files`.
    pub unused_segments: AHashMap<String, Vec<SegmentReport>>,
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
                None => writeln!(f, "{} is completely unused (has no exports)", file_path)?,
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
    graph: &ResultGraph,
    include_symbol: impl Fn(&ResultFile, &ExportedSymbol) -> Option<T> + Sync,
) -> AHashMap<String, Vec<T>> {
    graph
        .files
        .par_iter()
        .filter_map(|file| -> Option<(String, Vec<T>)> {
            // Find all used symbols in the file
            let unused_symbols = file
                .import_export_info
                .iter_exported_symbols()
                .filter_map(|(_, symbol): (_, &ExportedSymbol)| -> Option<T> {
                    include_symbol(file, symbol)
                })
                .collect::<Vec<_>>();

            if unused_symbols.is_empty() {
                return None;
            }

            Some((
                file.file_path.to_string_lossy().to_string(),
                unused_symbols,
            ))
        })
        .collect::<AHashMap<String, Vec<T>>>()
}

fn is_used(tags: &UsedTag, config: &UnusedFinderConfig) -> bool {
    tags.contains(UsedTag::FROM_ENTRY)
        || tags.contains(UsedTag::FROM_IGNORED)
        || tags.contains(UsedTag::FROM_TEST)
        || (config.allow_unused_types && tags.contains(UsedTag::TYPE_ONLY))
}
fn include_extra(tags: &UsedTag) -> bool {
    !tags.is_empty() && *tags != UsedTag::FROM_ENTRY
}

/// Build a `SegmentGraph` from the file-level `Graph`, then seed and propagate
/// reachability based on file/symbol tags from the BFS traversal.
///
/// Returns a map from file path → list of `SegmentReport` for unused segments
/// in files that are at least partially used.
fn compute_unused_segments(graph: &ResultGraph, config: &UnusedFinderConfig) -> AHashMap<String, Vec<SegmentReport>> {
    // Collect file_segments for the SegmentGraph builder.
    let file_segments: Vec<(usize, &[Segment])> = graph
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.segments.is_empty())
        .map(|(file_id, f)| (file_id, f.segments.as_slice()))
        .collect();

    if file_segments.is_empty() {
        return AHashMap::default();
    }

    let mut seg_graph = SegmentGraph::build(&file_segments);

    // Build resolved_imports: (importing_file_id, specifier_string) → target_file_id.
    // We can derive this from the existing resolved import_export_info, which stores
    // resolved PathBuf keys in its import maps.
    let mut resolved_imports: AHashMap<(usize, String), usize> = AHashMap::default();
    for (file_id, graph_file) in graph.files.iter().enumerate() {
        for seg in &graph_file.segments {
            // Static imports
            for specifier in seg.module_deps.imports.keys() {
                if let Some(&target_id) = graph.path_to_id.get(std::path::Path::new(specifier)) {
                    resolved_imports.insert((file_id, specifier.clone()), target_id);
                }
            }
            // Dynamic imports
            for specifier in seg.module_deps.dynamic_imports.keys() {
                if let Some(&target_id) = graph.path_to_id.get(std::path::Path::new(specifier)) {
                    resolved_imports.insert((file_id, specifier.clone()), target_id);
                }
            }
            // Re-exports
            for specifier in seg.module_deps.exports_from.keys() {
                if let Some(&target_id) = graph.path_to_id.get(std::path::Path::new(specifier)) {
                    resolved_imports.insert((file_id, specifier.clone()), target_id);
                }
            }
            // Side-effect imports
            for specifier in &seg.module_deps.executed_paths {
                if let Some(&target_id) = graph.path_to_id.get(std::path::Path::new(specifier)) {
                    resolved_imports.insert((file_id, specifier.clone()), target_id);
                }
            }
        }
    }

    seg_graph.add_inter_file_edges(&file_segments, &resolved_imports);

    // Seed reachability: for each file that is "used", mark its used segments.
    let mut seed_indices: Vec<usize> = Vec::new();
    for (file_id, graph_file) in graph.files.iter().enumerate() {
        if !is_used(&graph_file.file_tags, config) {
            // File is unused — don't seed any of its segments.
            continue;
        }

        // If the file is used as a namespace/entrypoint, all its segments are seeded.
        let all_symbols_used = graph_file
            .import_export_info
            .iter_exported_symbols()
            .all(|(_, sym)| {
                let default: UsedTag = Default::default();
                let tags = graph_file.symbol_tags.get(sym).unwrap_or(&default);
                is_used(tags, config)
            });

        if all_symbols_used || graph_file.segments.is_empty() {
            // All symbols used (or no segments) — seed all segments in this file.
            for seg_idx in 0..graph_file.segments.len() {
                if let Some(idx) = seg_graph.node_index(&SegmentId::new(file_id, seg_idx)) {
                    seed_indices.push(idx);
                }
            }
        } else {
            // Partially used file — seed only segments whose exported symbols are used.
            // Also seed segments that have no exports (side-effect statements) since
            // they execute when the file is imported.
            for (seg_idx, seg) in graph_file.segments.iter().enumerate() {
                let has_exports = !seg.module_deps.exports_locals.is_empty()
                    || !seg.module_deps.exports_from.is_empty();
                let has_side_effects = !seg.module_deps.executed_paths.is_empty()
                    || !seg.module_deps.requires.is_empty();

                let should_seed = if has_exports {
                    // Seed if any of its exported symbols is used.
                    seg.module_deps.exports_locals.keys().any(|exported_sym| {
                        let uf_sym = ExportedSymbol::from(exported_sym);
                        let default: UsedTag = Default::default();
                        let tags = graph_file.symbol_tags.get(&uf_sym).unwrap_or(&default);
                        is_used(tags, config)
                    })
                } else if has_side_effects {
                    // Side-effect imports always execute — seed them.
                    true
                } else {
                    // Non-exporting, non-side-effect segment (plain statement).
                    // These are reachable only via intra-file effect edges —
                    // they'll be reached by propagation if a later segment is seeded.
                    false
                };

                if should_seed {
                    if let Some(idx) = seg_graph.node_index(&SegmentId::new(file_id, seg_idx)) {
                        seed_indices.push(idx);
                    }
                }
            }
        }
    }

    seg_graph.propagate_tags(&seed_indices, TagSet::REACHABLE);

    // Collect unused segments for files that are at least partially used.
    let mut result: AHashMap<String, Vec<SegmentReport>> = AHashMap::default();
    for (file_id, graph_file) in graph.files.iter().enumerate() {
        if !is_used(&graph_file.file_tags, config) {
            continue; // Skip fully unused files.
        }

        let mut unused_segs: Vec<SegmentReport> = Vec::new();
        for (seg_idx, seg) in graph_file.segments.iter().enumerate() {
            if let Some(node_idx) = seg_graph.node_index(&SegmentId::new(file_id, seg_idx)) {
                if !seg_graph.node(node_idx).tags.contains(TagSet::REACHABLE) {
                    unused_segs.push(SegmentReport {
                        segment_idx: seg_idx,
                        start: seg.span.lo().to_u32(),
                        end: seg.span.hi().to_u32(),
                    });
                }
            }
        }

        if !unused_segs.is_empty() {
            unused_segs.sort();
            result.insert(
                graph_file.file_path.to_string_lossy().to_string(),
                unused_segs,
            );
        }
    }

    result
}

impl From<&UnusedFinderResult> for UnusedFinderReport {
    fn from(value: &UnusedFinderResult) -> Self {
        let mut unused_files: Vec<String> = value
            .graph
            .files
            .par_iter()
            .filter_map(|file| {
                if is_used(&file.file_tags, &value.config) {
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

                if is_used(symbol_bitflags, &value.config) {
                    // don't return used symbols
                    return None;
                }

                let ast_symbol = file.import_export_info.get_exported_symbol(symbol_name)?;

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
                if !include_extra(symbol_bitflags) {
                    // don't return symbols that are used or symbols that are truly unused
                    return None;
                }

                let ast_symbol = file.import_export_info.get_exported_symbol(symbol_name)?;

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

        let unused_segments = compute_unused_segments(&value.graph, &value.config);

        UnusedFinderReport {
            unused_files,
            unused_symbols,
            // TODO collect tags from symbols are "used", but not
            // entrypoints into the project
            extra_file_tags,
            extra_symbol_tags,
            unused_segments,
        }
    }
}
