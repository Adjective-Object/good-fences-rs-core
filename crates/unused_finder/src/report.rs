use core::{
    convert::Into,
    option::Option::{None, Some},
};
use std::fmt::Display;

use ahashmap::AHashMap;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use source_graph::SegmentKey;

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
    /// Zero-based UTF-8 byte offset, inclusive.
    pub start: u32,
    /// Zero-based UTF-8 byte offset, exclusive.
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
    /// Zero-based UTF-8 byte offset, inclusive.
    pub start: u32,
    /// Zero-based UTF-8 byte offset, exclusive.
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

            Some((file.file_path.to_string_lossy().to_string(), unused_symbols))
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

/// Apply report roots and conservative statement-order policy to the same
/// structural graph used for symbol/file usage.
///
/// Returns a map from file path → list of `SegmentReport` for unused segments
/// in files that are at least partially used.
fn compute_unused_segments(
    graph: &ResultGraph,
    config: &UnusedFinderConfig,
) -> AHashMap<String, Vec<SegmentReport>> {
    let source = &graph.source_graph;
    let mut tags = tag_graph::TagGraph::new();
    let mut previous = AHashMap::default();
    for file in &graph.files {
        let Some(file_id) = source.file_id(&file.file_path) else {
            continue;
        };
        let mut last = None;
        for (i, segment) in file.segments.iter().enumerate() {
            let deps = &segment.module_deps;
            if deps.imports.is_empty()
                && deps.exports_from.is_empty()
                && deps.executed_paths.is_empty()
            {
                let key = SegmentKey::new(file_id, i as u32);
                if let Some(last) = last {
                    previous.insert(key, last);
                }
                last = Some(key);
            }
        }
    }

    // Seed reachability: for each file that is "used", mark its used segments.
    let mut seeds = Vec::new();
    for graph_file in &graph.files {
        let Some(file_id) = source.file_id(&graph_file.file_path) else {
            continue;
        };
        if !is_used(&graph_file.file_tags, config) {
            // File is unused — don't seed any of its segments.
            continue;
        }

        // If the file is used as a namespace/entrypoint, all its segments are seeded.
        let all_symbols_used =
            graph_file
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
                seeds.push(SegmentKey::new(file_id, seg_idx as u32));
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
                    let local_used = seg.module_deps.exports_locals.keys().any(|exported_sym| {
                        let uf_sym = ExportedSymbol::from(exported_sym);
                        let default: UsedTag = Default::default();
                        let tags = graph_file.symbol_tags.get(&uf_sym).unwrap_or(&default);
                        is_used(tags, config)
                    });
                    let reexport_used =
                        seg.module_deps
                            .exports_from
                            .values()
                            .flatten()
                            .any(|export| {
                                let exported = export.exported_as.as_ref().or({
                                    match &export.imported_as {
                                        ast_segmenter::ImportTarget::ExportedSymbol(name) => {
                                            Some(name)
                                        }
                                        _ => None,
                                    }
                                });
                                exported.is_some_and(|symbol| {
                                    graph_file
                                        .symbol_tags
                                        .get(&ExportedSymbol::from(symbol))
                                        .is_some_and(|tag| is_used(tag, config))
                                })
                            });
                    local_used || reexport_used
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
                    seeds.push(SegmentKey::new(file_id, seg_idx as u32));
                }
            }
        }
    }

    tags.propagate_tags_for_report(
        source,
        seeds,
        tag_graph::UsedTag::FROM_ENTRY,
        config.allow_unused_types,
        &previous,
    );

    // Collect unused segments for files that are at least partially used.
    let mut result: AHashMap<String, Vec<SegmentReport>> = AHashMap::default();
    for graph_file in &graph.files {
        let Some(file_id) = source.file_id(&graph_file.file_path) else {
            continue;
        };
        if !is_used(&graph_file.file_tags, config) {
            continue; // Skip fully unused files.
        }

        let mut unused_segs: Vec<SegmentReport> = Vec::new();
        for (seg_idx, seg) in graph_file.segments.iter().enumerate() {
            if tags
                .get_tag(SegmentKey::new(file_id, seg_idx as u32))
                .is_empty()
            {
                unused_segs.push(SegmentReport {
                    segment_idx: seg_idx,
                    start: seg.span.start,
                    end: seg.span.end,
                });
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
                    start: ast_symbol.span.start,
                    end: ast_symbol.span.end,
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
                        start: ast_symbol.span.start,
                        end: ast_symbol.span.end,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnusedFinder;
    use test_tmpdir::test_tmpdir;

    #[test]
    fn static_import_keeps_named_reexport_barrels_in_all_report_surfaces() {
        let tmpdir = test_tmpdir!(
            "package.json" => r#"{"name":"entrypoint","main":"./main.js","exports":{}}"#,
            "main.js" => "import { Consumer } from './barrel.js'; console.log(Consumer);",
            "barrel.js" => "export { Consumer } from './lib.js';",
            "lib.js" => "export const Consumer = 1;"
        );
        let logger = logger::StdioLogger::new();
        let config = UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().into_owned(),
            root_paths: vec![".".to_owned()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        };
        let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
        let result = finder.find_unused(&logger).unwrap();
        let report = UnusedFinderReport::from(&result);
        assert!(report.unused_files.is_empty(), "{report:?}");
        assert!(report.unused_symbols.is_empty(), "{report:?}");
        assert!(report.unused_segments.is_empty(), "{report:?}");
        assert!(result.graph.source_graph.edges().iter().any(|edge| {
            matches!(edge.to, source_graph::GraphNode::Segment(key) if result.graph.source_graph.file_path(key.file_id) == Some(tmpdir.root_join("barrel.js").as_path()))
        }));
    }

    #[test]
    fn partially_used_barrel_seeds_its_used_reexport_statement() {
        let tmpdir = test_tmpdir!(
            "package.json" => r#"{"name":"entrypoint","main":"./main.js","exports":{}}"#,
            "main.js" => "import { live } from './barrel.js';",
            "barrel.js" => "export { live } from './lib.js'; export const dead = 2;",
            "lib.js" => "export const live = 1;"
        );
        let logger = logger::StdioLogger::new();
        let config = UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().into_owned(),
            root_paths: vec![".".to_owned()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        };
        let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
        let report = finder.find_unused(&logger).unwrap().get_report();
        let barrel = tmpdir.root_join("barrel.js");
        let barrel = barrel.to_str().unwrap();
        assert!(report.unused_files.is_empty(), "{report:?}");
        assert_eq!(report.unused_symbols.len(), 1, "{report:?}");
        assert_eq!(report.unused_symbols[barrel][0].id, "dead");
        assert_eq!(report.unused_segments.len(), 1, "{report:?}");
        assert_eq!(
            report.unused_segments[barrel]
                .iter()
                .map(|segment| segment.segment_idx)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn relative_dependency_segments_use_resolved_paths() {
        for (entry, expected_unused) in [
            ("import { live } from './dep.js';", vec![1]),
            ("import('./dep.js').then(({ live }) => live);", vec![1]),
            ("export { live } from './dep.js';", vec![1]),
            ("import './dep.js';", vec![]),
            ("require('./dep.js');", vec![]),
        ] {
            let tmpdir = test_tmpdir!(
                "package.json" => r#"{"name":"entrypoint","main":"./main.js","exports":{}}"#,
                "main.js" => entry,
                "dep.js" => "export const live = 1;\nexport const dead = 2;"
            );
            let logger = logger::StdioLogger::new();
            let config = UnusedFinderConfig {
                repo_root: tmpdir.root().to_string_lossy().into_owned(),
                root_paths: vec![".".to_owned()],
                entry_packages: vec!["entrypoint"].try_into().unwrap(),
                ..Default::default()
            };
            let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
            let mut result = finder.find_unused(&logger).unwrap();
            let dep_path = tmpdir.root_join("dep.js");
            let dep_id = result.graph.path_to_id[&dep_path];
            assert!(is_used(
                &result.graph.files[dep_id].file_tags,
                &result.config
            ));

            // Isolate segment-edge propagation from the already computed symbol tags.
            result.graph.files[dep_id].symbol_tags.clear();
            // Reports must not reconstruct dependency edges from result metadata.
            for file in &mut result.graph.files {
                file.specifier_to_resolved.clear();
            }
            let unused = compute_unused_segments(&result.graph, &result.config);
            let actual: Vec<_> = unused
                .get(dep_path.to_str().unwrap())
                .into_iter()
                .flatten()
                .map(|seg| seg.segment_idx)
                .collect();
            assert_eq!(actual, expected_unused, "{entry}");
        }
    }
}
