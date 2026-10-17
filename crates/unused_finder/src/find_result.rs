use std::path::{Path, PathBuf};

use ahashmap::AHashMap;
use ast_segmenter::segment_info::Segment;

use crate::{
    parse::{ExportedSymbol, ResolvedImportExportInfo},
    tag::UsedTag,
};

/// Data-only per-file result from unused detection.
///
/// Contains the computed tags (from `TagGraph` + TYPE_ONLY marking) alongside
/// the import/export metadata needed for report generation and DOT graph output.
#[derive(Debug, Clone, Default)]
pub struct ResultFile {
    pub file_path: PathBuf,
    /// Union of all tag sources that reached this file.
    pub file_tags: UsedTag,
    /// Per-symbol tags for each exported symbol.
    pub symbol_tags: AHashMap<ExportedSymbol, UsedTag>,
    /// Import/export metadata from resolution.
    pub import_export_info: ResolvedImportExportInfo,
    /// Legacy resolution metadata; reporting traverses `ResultGraph::source_graph`.
    pub specifier_to_resolved: AHashMap<String, PathBuf>,
    /// Per-statement segments from ast_segmenter.
    pub segments: Vec<Segment>,
}

/// Data-only graph result with file-indexed access.
///
/// Shares the immutable structural graph with report generation; file indices
/// below are result-local and need not equal structural module IDs.
#[derive(Default, Debug, Clone)]
pub struct ResultGraph {
    /// The authoritative graph used to compute these tags and segment reports.
    pub source_graph: std::sync::Arc<source_graph::SourceGraph>,
    pub path_to_id: AHashMap<PathBuf, usize>,
    pub files: Vec<ResultFile>,
}

impl ResultGraph {
    pub fn get_file_by_path(&self, path: &Path) -> Option<&ResultFile> {
        self.path_to_id.get(path).map(|id| &self.files[*id])
    }
}
