pub mod edge;
pub mod segment_key;
mod structure;
pub use structure::*;
mod linker;
pub use linker::*;
mod revision;
pub use revision::*;
mod legacy;
#[cfg(test)]
mod linker_tests;
#[cfg(test)]
mod revision_tests;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ahashmap::AHashMap;
use ast_segmenter::segment_info::Segment;
use ast_segmenter::variables::HoistingLevel;
use ast_segmenter::ExportedSymbol;
use compact_str::CompactString;

pub use edge::SegmentEdge;
pub use segment_key::SegmentKey;

/// Per-file data owned by the SourceGraph.
/// Pre-built indexes enable fast intra-file name resolution.
#[derive(Debug)]
struct SourceGraphFile {
    owned: Option<OwnedSourceFileInput>,
    path: PathBuf,
    segments: Vec<Segment>,
    /// Exported symbol → segment index that declares the export
    symbol_to_segment: AHashMap<ExportedSymbol, u32>,
    /// Local name → Vec<(segment_idx, HoistingLevel)> for intra-file lookup
    name_to_declaring_segments: AHashMap<CompactString, Vec<(u32, HoistingLevel)>>,
    /// Resolved import paths: raw specifier → resolved file path.
    /// Used during tag propagation to follow inter-file import edges.
    resolved_import_paths: AHashMap<String, PathBuf>,
}

/// Segment-level import graph.
///
/// Maps files to their segments and provides indexes for resolving
/// names to segments within and across files.
#[derive(Debug)]
pub struct SourceGraph {
    revision: GraphRevision,
    diagnostics: Vec<LinkDiagnostic>,
    edges: Vec<OccurrenceEdge>,
    outgoing: AHashMap<GraphNode, Vec<usize>>,
    incoming: AHashMap<GraphNode, Vec<usize>>,
    path_to_id: AHashMap<PathBuf, u32>,
    files: Vec<SourceGraphFile>,
    /// Cache for cross-file import resolution. Keyed by (file_id, symbol).
    /// Interior-mutated so `resolve_import_across_files` can take `&self`.
    reexport_cache: Mutex<AHashMap<(u32, ExportedSymbol), Vec<SegmentKey>>>,
}

/// Input data for constructing a [`SourceGraph`].
///
/// This mirrors the relevant fields of `unused_finder::ResolvedSourceFile`
/// without creating a crate dependency on `unused_finder`.
pub struct SourceFileInput {
    pub source_file_path: PathBuf,
    pub segments: Vec<Segment>,
    /// Maps raw import specifiers (keys of `exports_from` in segment `RawModuleDeps`)
    /// to resolved file paths. Required for cross-file re-export resolution.
    pub resolved_reexport_paths: AHashMap<String, PathBuf>,
    /// Maps raw import specifiers (keys of `imports`, `dynamic_imports`, `requires`,
    /// `executed_paths` in segment `RawModuleDeps`) to resolved file paths.
    /// Required for inter-file import edge traversal during tag propagation.
    pub resolved_import_paths: AHashMap<String, PathBuf>,
}

impl Default for SourceGraph {
    fn default() -> Self {
        Self::new(std::iter::empty())
    }
}

impl SourceGraph {
    /// Build a new SourceGraph from an iterator of source file inputs.
    ///
    /// For each file, builds:
    /// - `symbol_to_segment`: maps each exported symbol to the segment that exports it
    /// - `name_to_declaring_segments`: maps each locally declared name to its
    ///   declaring segment(s) with hoisting levels
    /// - `named_reexports` / `star_reexport_paths`: re-export indexes for cross-file resolution
    pub fn new(source_files: impl Iterator<Item = SourceFileInput>) -> Self {
        let mut path_to_id = AHashMap::default();
        let mut files = Vec::new();

        for input in source_files {
            let file_id = files.len() as u32;
            path_to_id.insert(input.source_file_path.clone(), file_id);

            let file = Self::build_file(
                input.source_file_path,
                input.segments,
                input.resolved_reexport_paths,
                input.resolved_import_paths,
            );
            files.push(file);
        }

        let mut graph = SourceGraph {
            revision: GraphRevision::default(),
            diagnostics: Vec::new(),
            edges: Vec::new(),
            outgoing: AHashMap::default(),
            incoming: AHashMap::default(),
            path_to_id,
            files,
            reexport_cache: Mutex::new(AHashMap::default()),
        };
        graph.rebuild_edges();
        graph
    }

    fn build_file(
        path: PathBuf,
        segments: Vec<Segment>,
        resolved_reexport_paths: AHashMap<String, PathBuf>,
        resolved_import_paths: AHashMap<String, PathBuf>,
    ) -> SourceGraphFile {
        let mut symbol_to_segment: AHashMap<ExportedSymbol, u32> = AHashMap::default();
        let mut name_to_declaring_segments: AHashMap<CompactString, Vec<(u32, HoistingLevel)>> =
            AHashMap::default();

        // Merge resolved import and reexport paths so TagGraph can resolve both
        let mut all_resolved_paths = resolved_import_paths;
        all_resolved_paths.extend(
            resolved_reexport_paths
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );

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
            owned: None,
            path,
            segments,
            symbol_to_segment,
            name_to_declaring_segments,
            resolved_import_paths: all_resolved_paths,
        }
    }

    /// Returns the file_id for a given path, if it exists in the graph.
    pub fn file_id(&self, path: &Path) -> Option<u32> {
        self.path_to_id.get(path).copied()
    }

    /// Returns the number of files in the graph.
    pub fn file_count(&self) -> usize {
        self.path_to_id.len()
    }

    /// Live module identities, including empty modules and excluding tombstones.
    pub fn module_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.files.iter().enumerate().filter_map(|(id, file)| {
            let id = id as u32;
            (self.file_id(&file.path) == Some(id)).then_some(id)
        })
    }

    /// Returns the path of a file by its id.
    pub fn file_path(&self, file_id: u32) -> Option<&Path> {
        self.files
            .get(file_id as usize)
            .filter(|f| self.file_id(&f.path) == Some(file_id))
            .map(|f| f.path.as_path())
    }

    /// Returns the segments for a file by its id.
    pub fn file_segments(&self, file_id: u32) -> Option<&[Segment]> {
        self.file_path(file_id)?;
        self.files
            .get(file_id as usize)
            .map(|f| f.segments.as_slice())
    }

    /// Returns the symbol_to_segment index for a file.
    pub fn file_symbol_to_segment(&self, file_id: u32) -> Option<&AHashMap<ExportedSymbol, u32>> {
        self.files
            .get(file_id as usize)
            .map(|f| &f.symbol_to_segment)
    }

    /// Returns the resolved import paths for a file.
    pub fn file_resolved_import_paths(&self, file_id: u32) -> Option<&AHashMap<String, PathBuf>> {
        self.files
            .get(file_id as usize)
            .map(|f| &f.resolved_import_paths)
    }

    /// Resolve a name reference to the segment that declares it within the same file.
    ///
    /// Owned inputs use lexical binding IDs (including nested shadows). Legacy
    /// top-level names bind even before var/lexical declarations; TDZ is not
    /// name-resolution failure.
    pub fn resolve_symbol_in_file(
        &self,
        file_id: u32,
        name: &str,
        referencing_segment_idx: u32,
    ) -> Option<SegmentKey> {
        let file = self.files.get(file_id as usize)?;
        if let Some(input) = &file.owned {
            let span = file.segments.get(referencing_segment_idx as usize)?.span;
            let binding = input
                .facts
                .references
                .iter()
                .find(|r| r.name == name && span.start <= r.span.start && r.span.end <= span.end)
                .and_then(|r| r.binding)
                .or_else(|| {
                    input
                        .facts
                        .bindings
                        .iter()
                        .find(|b| b.top_level && b.name == name)
                        .map(|b| b.id)
                })?;
            return match self.resolve_binding(file_id, binding)? {
                GraphNode::Segment(key) => Some(key),
                _ => None,
            };
        }
        let declarations = file.name_to_declaring_segments.get(name)?;

        // TDZ constrains evaluation, not lexical binding.
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
                    if best_let_const.map_or(true, |prev| seg_idx < prev) {
                        best_let_const = Some(seg_idx);
                    }
                }
            }
        }

        let chosen = best_let_const.or(best_hoisted)?;
        Some(SegmentKey::new(file_id, chosen))
    }

    /// Resolve an import of `symbol` from `target_file_id` to the segment(s)
    /// that ultimately declare/export it. Follows re-export chains lazily,
    /// caching results. Returns empty Vec if the symbol is not found.
    ///
    /// Compatibility adapter includes type exports and returns empty on errors.
    /// Use `resolve_export` for explicit namespaces, routes and diagnostics.
    pub fn resolve_import_across_files(
        &self,
        target_file_id: u32,
        symbol: &ExportedSymbol,
    ) -> Vec<SegmentKey> {
        // Check cache first
        let cache_key = (target_file_id, symbol.clone());
        if let Some(cached) = self
            .reexport_cache
            .lock()
            .expect("export cache")
            .get(&cache_key)
        {
            return cached.clone();
        }

        let result = self
            .resolve_export(target_file_id, exported_name(symbol), Namespace::Type)
            .map(|resolved| match resolved.declaration {
                GraphNode::Segment(key) => vec![key],
                GraphNode::Module(id) => self.files[id as usize]
                    .segments
                    .iter()
                    .enumerate()
                    .map(|(i, _)| SegmentKey::new(id, i as u32))
                    .collect(),
                GraphNode::Unlinked { .. } => vec![],
            })
            .unwrap_or_default();

        // Cache the result
        self.reexport_cache
            .lock()
            .expect("export cache")
            .insert(cache_key, result.clone());
        result
    }

    /// Clear the re-export resolution cache.
    /// Should be called when file data changes (e.g., via `patch_file`).
    pub fn clear_reexport_cache(&self) {
        self.reexport_cache.lock().expect("export cache").clear();
    }

    /// Iterate over segments in a specific file, yielding `(SegmentKey, &Segment)` pairs.
    pub fn iter_file_segments(
        &self,
        path: &Path,
    ) -> Option<impl Iterator<Item = (SegmentKey, &Segment)>> {
        let &file_id = self.path_to_id.get(path)?;
        let file = &self.files[file_id as usize];
        Some(
            file.segments
                .iter()
                .enumerate()
                .map(move |(idx, seg)| (SegmentKey::new(file_id, idx as u32), seg)),
        )
    }

    /// Iterate over all segments in the graph, yielding `(SegmentKey, &Segment)` pairs.
    pub fn iter_segments(&self) -> impl Iterator<Item = (SegmentKey, &Segment)> {
        self.files.iter().enumerate().flat_map(|(file_id, file)| {
            let file_id = file_id as u32;
            file.segments
                .iter()
                .enumerate()
                .map(move |(idx, seg)| (SegmentKey::new(file_id, idx as u32), seg))
        })
    }

    /// Replace a single file's data in-place. Rebuilds that file's indexes
    /// and invalidates the re-export resolution cache.
    ///
    /// Uses `SourceFileInput` instead of `ResolvedSourceFile` to avoid a
    /// circular crate dependency with `unused_finder`.
    pub fn patch_file(&mut self, path: &Path, input: SourceFileInput) -> eyre::Result<()> {
        eyre::ensure!(
            path == input.source_file_path,
            "patch path mismatch; rename requires remove plus add"
        );
        // Clear the entire re-export cache — any cached resolution could
        // transitively depend on the patched file's exports.
        self.reexport_cache.lock().expect("export cache").clear();

        if let Some(&file_id) = self.path_to_id.get(path) {
            // Existing file: rebuild in place
            let new_file = Self::build_file(
                input.source_file_path,
                input.segments,
                input.resolved_reexport_paths,
                input.resolved_import_paths,
            );
            self.files[file_id as usize] = new_file;
        } else {
            // New file: append and update path_to_id
            let file_id = self.files.len() as u32;
            self.path_to_id
                .insert(input.source_file_path.clone(), file_id);
            let new_file = Self::build_file(
                input.source_file_path,
                input.segments,
                input.resolved_reexport_paths,
                input.resolved_import_paths,
            );
            self.files.push(new_file);
        }
        self.revision.source += 1;
        self.revision.resolution += 1;
        self.rebuild_edges();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_segmenter::raw_module_deps::RawModuleDeps;
    use ast_segmenter::variables::VariableScope;
    use ast_segmenter::ImportTarget;
    use oxc_span::Span;

    fn make_span(lo: u32, hi: u32) -> Span {
        Span::new(lo, hi)
    }

    fn simple_segment() -> Segment {
        Segment {
            module_deps: RawModuleDeps::default(),
            variables: VariableScope::new(),
            span: make_span(0, 10),
        }
    }

    fn simple_input(path: PathBuf, segments: Vec<Segment>) -> SourceFileInput {
        SourceFileInput {
            source_file_path: path,
            segments,
            resolved_reexport_paths: AHashMap::default(),
            resolved_import_paths: AHashMap::default(),
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

    /// Build a segment with a re-export: `export { imported } from 'specifier'`
    /// or `export { imported as exported } from 'specifier'`
    fn segment_with_reexport(
        specifier: &str,
        imported_as: ImportTarget,
        exported_as: Option<ExportedSymbol>,
    ) -> Segment {
        use ast_segmenter::raw_module_deps::SymbolTags;
        let mut deps = RawModuleDeps::default();
        let reexport = ast_segmenter::ReExportedSymbol {
            imported_as,
            exported_as,
            tags: SymbolTags::default(),
            span: make_span(0, 10),
        };
        deps.exports_from
            .entry(specifier.to_string())
            .or_default()
            .insert(reexport);

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
        let graph =
            SourceGraph::new(vec![simple_input(path.clone(), vec![simple_segment()])].into_iter());

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
                simple_input(path_a.clone(), vec![simple_segment()]),
                simple_input(path_b.clone(), vec![simple_segment(), simple_segment()]),
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
            vec![simple_input(
                path.clone(),
                vec![simple_segment(), segment_with_export("foo")],
            )]
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
        vars.insert_local_str(name, hoisting);
        Segment {
            module_deps: RawModuleDeps::default(),
            variables: vars,
            span: make_span(0, 10),
        }
    }

    // -- resolve_symbol_in_file tests --

    #[test]
    fn test_resolve_import_hoisted_from_later_segment() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![simple_input(
                path,
                vec![
                    segment_with_local("foo", HoistingLevel::ImportHoisting), // seg 0
                    simple_segment(),                                         // seg 1
                    simple_segment(),                                         // seg 2
                ],
            )]
            .into_iter(),
        );

        assert_eq!(
            graph.resolve_symbol_in_file(0, "foo", 2),
            Some(SegmentKey::new(0, 0))
        );
    }

    #[test]
    fn test_resolve_function_hoisted_from_earlier_segment() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![simple_input(
                path,
                vec![
                    simple_segment(),                                           // seg 0
                    simple_segment(),                                           // seg 1
                    segment_with_local("bar", HoistingLevel::FunctionHoisting), // seg 2
                ],
            )]
            .into_iter(),
        );

        assert_eq!(
            graph.resolve_symbol_in_file(0, "bar", 0),
            Some(SegmentKey::new(0, 2))
        );
    }

    #[test]
    fn test_resolve_let_const_is_bound_before_declaration() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![simple_input(
                path,
                vec![
                    simple_segment(),                                           // seg 0
                    simple_segment(),                                           // seg 1
                    segment_with_local("baz", HoistingLevel::LetConstHoisting), // seg 2
                ],
            )]
            .into_iter(),
        );

        assert_eq!(
            graph.resolve_symbol_in_file(0, "baz", 0),
            Some(SegmentKey::new(0, 2))
        );
        assert_eq!(
            graph.resolve_symbol_in_file(0, "baz", 1),
            Some(SegmentKey::new(0, 2))
        );
        assert_eq!(
            graph.resolve_symbol_in_file(0, "baz", 2),
            Some(SegmentKey::new(0, 2))
        );
    }

    #[test]
    fn test_resolve_name_not_found_returns_none() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(vec![simple_input(path, vec![simple_segment()])].into_iter());

        assert_eq!(graph.resolve_symbol_in_file(0, "nonexistent", 0), None);
        assert_eq!(graph.resolve_symbol_in_file(99, "anything", 0), None);
    }

    // -- resolve_import_across_files tests --

    #[test]
    fn test_cross_file_direct_export_resolves() {
        // File B exports "foo" from segment 1.
        // Resolving "foo" in file B should return SegmentKey(1, 1).
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let graph = SourceGraph::new(
            vec![
                simple_input(path_a, vec![simple_segment()]),
                simple_input(path_b, vec![simple_segment(), segment_with_export("foo")]),
            ]
            .into_iter(),
        );

        let result = graph.resolve_import_across_files(1, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![SegmentKey::new(1, 1)]);
    }

    #[test]
    fn test_cross_file_reexport_chain() {
        // A re-exports foo from B: `export { foo } from './b'`
        // B re-exports foo from C: `export { foo } from './c'`
        // C directly exports foo from segment 0.
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let foo_sym = ExportedSymbol::from("foo");

        let graph = SourceGraph::new(
            vec![
                // A: re-exports foo from B
                SourceFileInput {
                    source_file_path: path_a,
                    segments: vec![segment_with_reexport(
                        "./b",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), PathBuf::from("/test/b.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                // B: re-exports foo from C
                SourceFileInput {
                    source_file_path: path_b,
                    segments: vec![segment_with_reexport(
                        "./c",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./c".to_string(), PathBuf::from("/test/c.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                // C: directly exports foo
                simple_input(
                    PathBuf::from("/test/c.ts"),
                    vec![segment_with_export("foo")],
                ),
            ]
            .into_iter(),
        );

        // Resolving foo from A should follow A → B → C and return segment in C
        let result = graph.resolve_import_across_files(0, &foo_sym);
        assert_eq!(result, vec![SegmentKey::new(2, 0)]);
    }

    #[test]
    fn test_cross_file_star_reexport_fanout() {
        // A: `export * from './b'`
        // B: exports "foo" (segment 0) and "bar" (segment 1)
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let graph = SourceGraph::new(
            vec![
                // A: star re-export from B
                SourceFileInput {
                    source_file_path: path_a,
                    segments: vec![segment_with_reexport("./b", ImportTarget::Namespace, None)],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), PathBuf::from("/test/b.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                // B: exports foo and bar
                simple_input(
                    path_b,
                    vec![segment_with_export("foo"), segment_with_export("bar")],
                ),
            ]
            .into_iter(),
        );

        // Resolving "foo" from A should find it in B
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);

        // Resolving "bar" from A should also find it in B
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("bar"));
        assert_eq!(result, vec![SegmentKey::new(1, 1)]);

        // Resolving "baz" (not exported by B) should return empty
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("baz"));
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_cross_file_cycle_terminates() {
        // A re-exports foo from B, B re-exports foo from A → cycle
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");
        let foo_sym = ExportedSymbol::from("foo");

        let graph = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: path_a,
                    segments: vec![segment_with_reexport(
                        "./b",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), PathBuf::from("/test/b.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                SourceFileInput {
                    source_file_path: path_b,
                    segments: vec![segment_with_reexport(
                        "./a",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./a".to_string(), PathBuf::from("/test/a.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        // Should not panic, should return empty (no direct export found)
        let result = graph.resolve_import_across_files(0, &foo_sym);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_cross_file_renamed_reexport() {
        // A: `export { foo as bar } from './b'`
        // B: directly exports "foo" in segment 0
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let foo_sym = ExportedSymbol::from("foo");
        let bar_sym = ExportedSymbol::from("bar");

        let graph = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: path_a,
                    segments: vec![segment_with_reexport(
                        "./b",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        Some(bar_sym.clone()),
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), PathBuf::from("/test/b.ts"));
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                simple_input(path_b, vec![segment_with_export("foo")]),
            ]
            .into_iter(),
        );

        // Resolving "bar" from A should follow to "foo" in B
        let result = graph.resolve_import_across_files(0, &bar_sym);
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);

        // Resolving "foo" from A should NOT match (it's exported as "bar")
        let result = graph.resolve_import_across_files(0, &foo_sym);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_cross_file_symbol_not_found() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![simple_input(path, vec![segment_with_export("foo")])].into_iter(),
        );

        // "bar" is not exported
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("bar"));
        assert_eq!(result, vec![]);

        // Invalid file_id
        let result = graph.resolve_import_across_files(99, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![]);
    }

    // -- iter_file_segments / iter_segments tests --

    #[test]
    fn test_iter_file_segments() {
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");
        let graph = SourceGraph::new(
            vec![
                simple_input(
                    path_a.clone(),
                    vec![simple_segment(), segment_with_export("foo")],
                ),
                simple_input(path_b.clone(), vec![segment_with_export("bar")]),
            ]
            .into_iter(),
        );

        // File A has 2 segments
        let pairs: Vec<_> = graph.iter_file_segments(&path_a).unwrap().collect();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, SegmentKey::new(0, 0));
        assert_eq!(pairs[1].0, SegmentKey::new(0, 1));

        // File B has 1 segment
        let pairs: Vec<_> = graph.iter_file_segments(&path_b).unwrap().collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, SegmentKey::new(1, 0));

        // Unknown file returns None
        assert!(graph.iter_file_segments(Path::new("/nope")).is_none());
    }

    #[test]
    fn test_iter_segments() {
        let graph = SourceGraph::new(
            vec![
                simple_input(
                    PathBuf::from("/test/a.ts"),
                    vec![simple_segment(), simple_segment()],
                ),
                simple_input(PathBuf::from("/test/b.ts"), vec![simple_segment()]),
            ]
            .into_iter(),
        );

        let keys: Vec<SegmentKey> = graph.iter_segments().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![
                SegmentKey::new(0, 0),
                SegmentKey::new(0, 1),
                SegmentKey::new(1, 0),
            ]
        );
    }

    // -- patch_file tests --

    #[test]
    fn test_patch_file_updates_resolution() {
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");

        let mut graph = SourceGraph::new(
            vec![
                simple_input(path_a.clone(), vec![segment_with_export("foo")]),
                simple_input(path_b.clone(), vec![segment_with_export("bar")]),
            ]
            .into_iter(),
        );

        // Before patch: file A exports "foo"
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![SegmentKey::new(0, 0)]);

        // Patch file A: now exports "baz" instead of "foo"
        graph
            .patch_file(
                &path_a,
                simple_input(path_a.clone(), vec![segment_with_export("baz")]),
            )
            .unwrap();

        // "foo" no longer resolves
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![]);

        // "baz" now resolves
        let result = graph.resolve_import_across_files(0, &ExportedSymbol::from("baz"));
        assert_eq!(result, vec![SegmentKey::new(0, 0)]);

        // File B unaffected
        let result = graph.resolve_import_across_files(1, &ExportedSymbol::from("bar"));
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);
    }

    #[test]
    fn test_patch_file_invalidates_reexport_cache() {
        // A re-exports foo from B. B exports foo.
        // Patch B to export "qux" instead — cached resolution should be invalidated.
        let path_a = PathBuf::from("/test/a.ts");
        let path_b = PathBuf::from("/test/b.ts");
        let foo_sym = ExportedSymbol::from("foo");

        let mut graph = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: path_a.clone(),
                    segments: vec![segment_with_reexport(
                        "./b",
                        ImportTarget::ExportedSymbol(foo_sym.clone()),
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), path_b.clone());
                        m
                    },
                    resolved_import_paths: AHashMap::default(),
                },
                simple_input(path_b.clone(), vec![segment_with_export("foo")]),
            ]
            .into_iter(),
        );

        // Populate cache
        let result = graph.resolve_import_across_files(0, &foo_sym);
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);

        // Patch B: now exports "qux" instead of "foo"
        graph
            .patch_file(
                &path_b,
                simple_input(path_b.clone(), vec![segment_with_export("qux")]),
            )
            .unwrap();

        // Cached result should be gone; re-export chain should now fail
        let result = graph.resolve_import_across_files(0, &foo_sym);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_patch_file_adds_new_file() {
        let path_a = PathBuf::from("/test/a.ts");
        let mut graph = SourceGraph::new(
            vec![simple_input(path_a.clone(), vec![simple_segment()])].into_iter(),
        );
        assert_eq!(graph.file_count(), 1);

        // Patch in a new file
        let path_b = PathBuf::from("/test/b.ts");
        graph
            .patch_file(
                &path_b,
                simple_input(path_b.clone(), vec![segment_with_export("new_sym")]),
            )
            .unwrap();

        assert_eq!(graph.file_count(), 2);
        assert_eq!(graph.file_id(&path_b), Some(1));
        let result = graph.resolve_import_across_files(1, &ExportedSymbol::from("new_sym"));
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);
    }

    #[test]
    fn test_patch_file_updates_iterators() {
        let path = PathBuf::from("/test/a.ts");
        let mut graph =
            SourceGraph::new(vec![simple_input(path.clone(), vec![simple_segment()])].into_iter());

        // Initially 1 segment
        assert_eq!(graph.iter_segments().count(), 1);
        assert_eq!(graph.iter_file_segments(&path).unwrap().count(), 1);

        // Patch with 3 segments
        graph
            .patch_file(
                &path,
                simple_input(
                    path.clone(),
                    vec![simple_segment(), simple_segment(), simple_segment()],
                ),
            )
            .unwrap();

        assert_eq!(graph.iter_segments().count(), 3);
        assert_eq!(graph.iter_file_segments(&path).unwrap().count(), 3);
    }
}
