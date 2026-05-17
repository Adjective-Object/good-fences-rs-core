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

    /// Resolve a name reference to the segment that declares it within the same file.
    ///
    /// Hoisting rules:
    /// - `ImportHoisting` / `FunctionHoisting`: visible to all segments in the file,
    ///   so the declaring segment is always a valid resolution target.
    /// - `LetConstHoisting`: visible only to segments at the same or later index.
    ///   When multiple `LetConst` declarations exist, prefer the latest one at or
    ///   before `referencing_segment_idx`.
    ///
    /// Returns `None` if the name is not declared in any reachable segment.
    pub fn resolve_symbol_in_file(
        &self,
        file_id: u32,
        name: &str,
        referencing_segment_idx: u32,
    ) -> Option<SegmentKey> {
        let file = self.files.get(file_id as usize)?;
        let atom = Atom::from(name);
        let declarations = file.name_to_declaring_segments.get(&atom)?;

        // Pick the best declaration according to hoisting rules.
        // Hoisted declarations (Import/Function) are always visible — take the first.
        // LetConst declarations are only visible at or after the declaring segment —
        // take the latest one at or before referencing_segment_idx.
        let mut best_hoisted: Option<u32> = None;
        let mut best_let_const: Option<u32> = None;

        for &(seg_idx, hoisting) in declarations {
            match hoisting {
                HoistingLevel::ImportHoisting | HoistingLevel::FunctionHoisting => {
                    // Hoisted: always visible, prefer the first declaration
                    if best_hoisted.map_or(true, |prev| seg_idx < prev) {
                        best_hoisted = Some(seg_idx);
                    }
                }
                HoistingLevel::LetConstHoisting => {
                    // Only visible from the declaring segment onward
                    if seg_idx <= referencing_segment_idx {
                        if best_let_const.map_or(true, |prev| seg_idx > prev) {
                            best_let_const = Some(seg_idx);
                        }
                    }
                }
            }
        }

        // LetConst shadows hoisted if both exist and LetConst is visible
        let chosen = best_let_const.or(best_hoisted)?;
        Some(SegmentKey::new(file_id, chosen))
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

    fn segment_with_local(name: &str, hoisting: HoistingLevel) -> Segment {
        let mut vars = VariableScope::new();
        vars.insert_local(Atom::from(name), hoisting);
        Segment {
            module_deps: RawModuleDeps::default(),
            variables: vars,
            span: make_span(0, 10),
        }
    }

    // -- resolve_symbol_in_file tests --

    #[test]
    fn test_resolve_import_hoisted_from_later_segment() {
        // Import-hoisted name declared in segment 0, referenced from segment 2
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path,
                segments: vec![
                    segment_with_local("foo", HoistingLevel::ImportHoisting), // seg 0
                    simple_segment(),                                         // seg 1
                    simple_segment(),                                         // seg 2
                ],
            }]
            .into_iter(),
        );

        // Import hoisting: visible from any segment, including later ones
        assert_eq!(
            graph.resolve_symbol_in_file(0, "foo", 2),
            Some(SegmentKey::new(0, 0))
        );
    }

    #[test]
    fn test_resolve_function_hoisted_from_earlier_segment() {
        // Function-hoisted name declared in segment 2, referenced from segment 0
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path,
                segments: vec![
                    simple_segment(),                                           // seg 0
                    simple_segment(),                                           // seg 1
                    segment_with_local("bar", HoistingLevel::FunctionHoisting), // seg 2
                ],
            }]
            .into_iter(),
        );

        // Function hoisting: visible from earlier segments
        assert_eq!(
            graph.resolve_symbol_in_file(0, "bar", 0),
            Some(SegmentKey::new(0, 2))
        );
    }

    #[test]
    fn test_resolve_let_const_not_visible_from_earlier_segment() {
        // let/const declared in segment 2, referenced from segment 0
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path,
                segments: vec![
                    simple_segment(),                                            // seg 0
                    simple_segment(),                                            // seg 1
                    segment_with_local("baz", HoistingLevel::LetConstHoisting),  // seg 2
                ],
            }]
            .into_iter(),
        );

        // LetConst: NOT visible from earlier segments
        assert_eq!(graph.resolve_symbol_in_file(0, "baz", 0), None);
        assert_eq!(graph.resolve_symbol_in_file(0, "baz", 1), None);
        // But visible from the declaring segment and later
        assert_eq!(
            graph.resolve_symbol_in_file(0, "baz", 2),
            Some(SegmentKey::new(0, 2))
        );
    }

    #[test]
    fn test_resolve_name_not_found_returns_none() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: path,
                segments: vec![simple_segment()],
            }]
            .into_iter(),
        );

        assert_eq!(graph.resolve_symbol_in_file(0, "nonexistent", 0), None);
        // Also None for invalid file_id
        assert_eq!(graph.resolve_symbol_in_file(99, "anything", 0), None);
    }
}
