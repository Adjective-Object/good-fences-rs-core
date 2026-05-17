pub mod edge;
pub mod segment_key;

use std::path::{Path, PathBuf};

use ahashmap::AHashMap;
use ast_name_tracker::visitor::HoistingLevel;
use ast_segmenter::segment_info::Segment;
use ast_segmenter::ExportedSymbol;
use swc_atoms::Atom;

pub use edge::SegmentEdge;
pub use segment_key::SegmentKey;

/// Per-file data owned by the SourceGraph.
/// Pre-built indexes enable fast intra-file name resolution.
struct SourceGraphFile {
    path: PathBuf,
    segments: Vec<Segment>,
    /// Exported symbol → segment index that declares the export
    symbol_to_segment: AHashMap<ExportedSymbol, u32>,
    /// Local name → Vec<(segment_idx, HoistingLevel)> for intra-file lookup
    name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>>,
}

/// Segment-level import graph.
///
/// Maps files to their segments and provides indexes for resolving
/// names to segments within and across files.
pub struct SourceGraph {
    path_to_id: AHashMap<PathBuf, u32>,
    files: Vec<SourceGraphFile>,
}

/// Input data for constructing a [`SourceGraph`].
///
/// This mirrors the relevant fields of `unused_finder::ResolvedSourceFile`
/// without creating a crate dependency on `unused_finder`.
pub struct SourceFileInput {
    pub source_file_path: PathBuf,
    pub segments: Vec<Segment>,
}

impl SourceGraph {
    /// Build a new SourceGraph from an iterator of source file inputs.
    ///
    /// For each file, builds:
    /// - `symbol_to_segment`: maps each exported symbol to the segment that exports it
    /// - `name_to_declaring_segments`: maps each locally declared name to its
    ///   declaring segment(s) with hoisting levels
    pub fn new(source_files: impl Iterator<Item = SourceFileInput>) -> Self {
        let mut path_to_id = AHashMap::default();
        let mut files = Vec::new();

        for input in source_files {
            let file_id = files.len() as u32;
            path_to_id.insert(input.source_file_path.clone(), file_id);

            let file = Self::build_file(input.source_file_path, input.segments);
            files.push(file);
        }

        SourceGraph { path_to_id, files }
    }

    fn build_file(path: PathBuf, segments: Vec<Segment>) -> SourceGraphFile {
        let mut symbol_to_segment: AHashMap<ExportedSymbol, u32> = AHashMap::default();
        let mut name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>> =
            AHashMap::default();

        for (seg_idx, seg) in segments.iter().enumerate() {
            let seg_idx = seg_idx as u32;

            // Build symbol_to_segment from exports_locals
            for exported_sym in seg.module_deps.exports_locals.keys() {
                symbol_to_segment.insert(exported_sym.clone(), seg_idx);
            }

            // Build name_to_declaring_segments from local variable declarations
            for (atom, hoisting) in seg.variables.get_locals_with_hoisting() {
                name_to_declaring_segments
                    .entry(atom.clone())
                    .or_default()
                    .push((seg_idx, hoisting));
            }
        }

        SourceGraphFile {
            path,
            segments,
            symbol_to_segment,
            name_to_declaring_segments,
        }
    }

    /// Returns the file_id for a given path, if it exists in the graph.
    pub fn file_id(&self, path: &Path) -> Option<u32> {
        self.path_to_id.get(path).copied()
    }

    /// Returns the number of files in the graph.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Returns the path of a file by its id.
    pub fn file_path(&self, file_id: u32) -> Option<&Path> {
        self.files.get(file_id as usize).map(|f| f.path.as_path())
    }

    /// Returns the segments for a file by its id.
    pub fn file_segments(&self, file_id: u32) -> Option<&[Segment]> {
        self.files.get(file_id as usize).map(|f| f.segments.as_slice())
    }

    /// Returns the symbol_to_segment index for a file.
    pub fn file_symbol_to_segment(&self, file_id: u32) -> Option<&AHashMap<ExportedSymbol, u32>> {
        self.files.get(file_id as usize).map(|f| &f.symbol_to_segment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_name_tracker::visitor::VariableScope;
    use ast_segmenter::raw_module_deps::RawModuleDeps;
    use swc_common::{BytePos, Span};

    fn make_span(lo: u32, hi: u32) -> Span {
        Span::new(BytePos(lo), BytePos(hi))
    }

    fn simple_segment() -> Segment {
        Segment {
            module_deps: RawModuleDeps::default(),
            variables: VariableScope::new(),
            span: make_span(0, 10),
        }
    }

    fn segment_with_export(name: &str) -> Segment {
        use ast_segmenter::raw_module_deps::{SymbolTags, TaggedSymbol};
        let mut deps = RawModuleDeps::default();
        let sym = ast_segmenter::ExportedSymbol::from(name);
        let tagged = TaggedSymbol::new(
            ast_segmenter::raw_module_deps::Symbol::from(name),
            SymbolTags::default(),
        );
        deps.exports_locals.insert(sym, tagged);

        Segment {
            module_deps: deps,
            variables: VariableScope::new(),
            span: make_span(0, 10),
        }
    }

    #[test]
    fn test_new_empty() {
        let graph = SourceGraph::new(std::iter::empty());
        assert_eq!(graph.file_count(), 0);
    }

    #[test]
    fn test_new_single_file() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path.clone(),
                segments: vec![simple_segment()],
            }]
            .into_iter(),
        );

        assert_eq!(graph.file_count(), 1);
        assert_eq!(graph.file_id(&path), Some(0));
        assert_eq!(graph.file_path(0), Some(Path::new("/test/a.ts")));
        assert_eq!(graph.file_segments(0).unwrap().len(), 1);
    }

    #[test]
    fn test_new_multiple_files() {
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let graph = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: path_a.clone(),
                    segments: vec![simple_segment()],
                },
                SourceFileInput {
                    source_file_path: path_b.clone(),
                    segments: vec![simple_segment(), simple_segment()],
                },
            ]
            .into_iter(),
        );

        assert_eq!(graph.file_count(), 2);
        assert_eq!(graph.file_id(&path_a), Some(0));
        assert_eq!(graph.file_id(&path_b), Some(1));
        assert_eq!(graph.file_segments(1).unwrap().len(), 2);
    }

    #[test]
    fn test_symbol_to_segment_index() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path.clone(),
                segments: vec![simple_segment(), segment_with_export("foo")],
            }]
            .into_iter(),
        );

        let idx = graph.file_symbol_to_segment(0).unwrap();
        let foo_sym = ast_segmenter::ExportedSymbol::from("foo");
        assert_eq!(idx.get(&foo_sym), Some(&1));
    }

    #[test]
    fn test_unknown_path_returns_none() {
        let graph = SourceGraph::new(std::iter::empty());
        assert_eq!(graph.file_id(Path::new("/nonexistent")), None);
        assert!(graph.file_path(99).is_none());
        assert!(graph.file_segments(99).is_none());
    }
}
