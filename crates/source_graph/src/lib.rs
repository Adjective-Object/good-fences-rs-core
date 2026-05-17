pub mod edge;
pub mod segment_key;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ahashmap::{AHashMap, AHashSet};
use ast_name_tracker::visitor::HoistingLevel;
use ast_segmenter::segment_info::Segment;
use ast_segmenter::{ExportedSymbol, ImportTarget};
use swc_atoms::Atom;

pub use edge::SegmentEdge;
pub use segment_key::SegmentKey;

/// A re-export target: the resolved path and symbol to look for in the target file.
#[derive(Debug, Clone)]
struct ReExportEntry {
    target_path: PathBuf,
    /// The symbol to resolve in the target file.
    imported_symbol: ExportedSymbol,
}

/// Per-file data owned by the SourceGraph.
/// Pre-built indexes enable fast intra-file name resolution.
struct SourceGraphFile {
    path: PathBuf,
    segments: Vec<Segment>,
    /// Exported symbol → segment index that declares the export
    symbol_to_segment: AHashMap<ExportedSymbol, u32>,
    /// Local name → Vec<(segment_idx, HoistingLevel)> for intra-file lookup
    name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>>,
    /// Named re-exports: exported symbol → list of (target path, symbol to look up in target)
    named_reexports: AHashMap<ExportedSymbol, Vec<ReExportEntry>>,
    /// Star re-export paths (`export * from`): resolved target paths
    star_reexport_paths: Vec<PathBuf>,
}

/// Segment-level import graph.
///
/// Maps files to their segments and provides indexes for resolving
/// names to segments within and across files.
pub struct SourceGraph {
    path_to_id: AHashMap<PathBuf, u32>,
    files: Vec<SourceGraphFile>,
    /// Cache for cross-file import resolution. Keyed by (file_id, symbol).
    /// Interior-mutated so `resolve_import_across_files` can take `&self`.
    reexport_cache: RefCell<AHashMap<(u32, ExportedSymbol), Vec<SegmentKey>>>,
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
            );
            files.push(file);
        }

        SourceGraph {
            path_to_id,
            files,
            reexport_cache: RefCell::new(AHashMap::default()),
        }
    }

    fn build_file(
        path: PathBuf,
        segments: Vec<Segment>,
        resolved_reexport_paths: AHashMap<String, PathBuf>,
    ) -> SourceGraphFile {
        let mut symbol_to_segment: AHashMap<ExportedSymbol, u32> = AHashMap::default();
        let mut name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>> =
            AHashMap::default();
        let mut named_reexports: AHashMap<ExportedSymbol, Vec<ReExportEntry>> =
            AHashMap::default();
        let mut star_reexport_paths: Vec<PathBuf> = Vec::new();

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

            // Build re-export indexes from exports_from
            for (specifier, reexports) in &seg.module_deps.exports_from {
                let target_path = match resolved_reexport_paths.get(specifier) {
                    Some(p) => p.clone(),
                    None => continue,
                };

                for reexport in reexports {
                    match (&reexport.imported_as, &reexport.exported_as) {
                        // Star re-export: `export * from './b'`
                        (ImportTarget::Namespace, None) => {
                            star_reexport_paths.push(target_path.clone());
                        }
                        // Named passthrough: `export { foo } from './b'`
                        (ImportTarget::ExportedSymbol(imported), None) => {
                            named_reexports
                                .entry(imported.clone())
                                .or_default()
                                .push(ReExportEntry {
                                    target_path: target_path.clone(),
                                    imported_symbol: imported.clone(),
                                });
                        }
                        // Renamed re-export: `export { foo as bar } from './b'`
                        (ImportTarget::ExportedSymbol(imported), Some(exported)) => {
                            named_reexports
                                .entry(exported.clone())
                                .or_default()
                                .push(ReExportEntry {
                                    target_path: target_path.clone(),
                                    imported_symbol: imported.clone(),
                                });
                        }
                        // Namespace under a name: `export * as Foo from './b'`
                        // Deferred — uncommon pattern, complex to resolve at segment level
                        (ImportTarget::Namespace, Some(_)) => {}
                    }
                }
            }
        }

        // Deduplicate star re-export paths
        star_reexport_paths.sort();
        star_reexport_paths.dedup();

        SourceGraphFile {
            path,
            segments,
            symbol_to_segment,
            name_to_declaring_segments,
            named_reexports,
            star_reexport_paths,
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

    /// Resolve an import of `symbol` from `target_file_id` to the segment(s)
    /// that ultimately declare/export it. Follows re-export chains lazily,
    /// caching results. Returns empty Vec if the symbol is not found.
    ///
    /// For star re-exports (`export * from`), fans out: returns a `SegmentKey`
    /// for each matching segment in the re-exported file.
    pub fn resolve_import_across_files(
        &self,
        target_file_id: u32,
        symbol: &ExportedSymbol,
    ) -> Vec<SegmentKey> {
        // Check cache first
        let cache_key = (target_file_id, symbol.clone());
        if let Some(cached) = self.reexport_cache.borrow().get(&cache_key) {
            return cached.clone();
        }

        let mut visited = AHashSet::default();
        let result = self.resolve_import_inner(target_file_id, symbol, &mut visited);

        // Cache the result
        self.reexport_cache
            .borrow_mut()
            .insert(cache_key, result.clone());
        result
    }

    /// Clear the re-export resolution cache.
    /// Should be called when file data changes (e.g., via `patch_file`).
    pub fn clear_reexport_cache(&self) {
        self.reexport_cache.borrow_mut().clear();
    }

    /// Inner recursive resolution with cycle detection.
    fn resolve_import_inner(
        &self,
        target_file_id: u32,
        symbol: &ExportedSymbol,
        visited: &mut AHashSet<(u32, ExportedSymbol)>,
    ) -> Vec<SegmentKey> {
        // Cycle detection
        if !visited.insert((target_file_id, symbol.clone())) {
            return vec![];
        }

        let file = match self.files.get(target_file_id as usize) {
            Some(f) => f,
            None => return vec![],
        };

        // 1. Direct export: symbol found in this file's exports_locals
        if let Some(&seg_idx) = file.symbol_to_segment.get(symbol) {
            return vec![SegmentKey::new(target_file_id, seg_idx)];
        }

        // 2. Named re-exports: explicit `export { x } from` or `export { x as y } from`
        if let Some(entries) = file.named_reexports.get(symbol) {
            let mut results = Vec::new();
            for entry in entries {
                if let Some(&next_file_id) = self.path_to_id.get(&entry.target_path) {
                    results.extend(self.resolve_import_inner(
                        next_file_id,
                        &entry.imported_symbol,
                        visited,
                    ));
                }
            }
            if !results.is_empty() {
                return results;
            }
        }

        // 3. Star re-exports: `export * from` — try each star source
        let mut results = Vec::new();
        for star_path in &file.star_reexport_paths {
            if let Some(&next_file_id) = self.path_to_id.get(star_path) {
                results.extend(self.resolve_import_inner(next_file_id, symbol, visited));
            }
        }
        results
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

    fn simple_input(path: PathBuf, segments: Vec<Segment>) -> SourceFileInput {
        SourceFileInput {
            source_file_path: path,
            segments,
            resolved_reexport_paths: AHashMap::default(),
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
        let graph = SourceGraph::new(
            vec![simple_input(path.clone(), vec![simple_segment()])].into_iter(),
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
    fn test_resolve_let_const_not_visible_from_earlier_segment() {
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

        assert_eq!(graph.resolve_symbol_in_file(0, "baz", 0), None);
        assert_eq!(graph.resolve_symbol_in_file(0, "baz", 1), None);
        assert_eq!(
            graph.resolve_symbol_in_file(0, "baz", 2),
            Some(SegmentKey::new(0, 2))
        );
    }

    #[test]
    fn test_resolve_name_not_found_returns_none() {
        let path = PathBuf::from("/test/a.ts");
        let graph = SourceGraph::new(
            vec![simple_input(path, vec![simple_segment()])].into_iter(),
        );

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

        let result =
            graph.resolve_import_across_files(1, &ExportedSymbol::from("foo"));
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
                },
                // C: directly exports foo
                simple_input(PathBuf::from("/test/c.ts"), vec![segment_with_export("foo")]),
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
                    segments: vec![segment_with_reexport(
                        "./b",
                        ImportTarget::Namespace,
                        None,
                    )],
                    resolved_reexport_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), PathBuf::from("/test/b.ts"));
                        m
                    },
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
        let result =
            graph.resolve_import_across_files(0, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![SegmentKey::new(1, 0)]);

        // Resolving "bar" from A should also find it in B
        let result =
            graph.resolve_import_across_files(0, &ExportedSymbol::from("bar"));
        assert_eq!(result, vec![SegmentKey::new(1, 1)]);

        // Resolving "baz" (not exported by B) should return empty
        let result =
            graph.resolve_import_across_files(0, &ExportedSymbol::from("baz"));
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
        let result =
            graph.resolve_import_across_files(0, &ExportedSymbol::from("bar"));
        assert_eq!(result, vec![]);

        // Invalid file_id
        let result =
            graph.resolve_import_across_files(99, &ExportedSymbol::from("foo"));
        assert_eq!(result, vec![]);
    }
}
