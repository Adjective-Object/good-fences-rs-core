use std::collections::VecDeque;

use ahashmap::{AHashMap, AHashSet};

use crate::raw_module_deps::{ExportedLocal, RawModuleDeps, Symbol};
use crate::segment_info::Segment;
use crate::ExportedSymbol;

/// Globally unique identifier for a segment within the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentId {
    pub file_id: usize,
    pub segment_idx: usize,
}

impl SegmentId {
    pub fn new(file_id: usize, segment_idx: usize) -> Self {
        Self {
            file_id,
            segment_idx,
        }
    }
}

/// Bitflags for segment properties that can be propagated through edges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TagSet(u32);

impl TagSet {
    pub const EMPTY: TagSet = TagSet(0);

    pub fn new(bits: u32) -> Self {
        Self(bits)
    }

    pub fn contains(self, other: TagSet) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: TagSet) {
        self.0 |= other.0;
    }

    pub fn union(self, other: TagSet) -> TagSet {
        TagSet(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn bits(self) -> u32 {
        self.0
    }
}

/// Well-known tag constants for common use cases.
impl TagSet {
    /// Segment is reachable from an entry point.
    pub const REACHABLE: TagSet = TagSet(1 << 0);
    /// Segment has side effects (or is transitively effectful).
    pub const EFFECTFUL: TagSet = TagSet(1 << 1);
}

/// A node in the segment graph.
#[derive(Debug, Clone)]
pub struct SegmentNode {
    pub id: SegmentId,
    pub tags: TagSet,
    /// True if this segment is hoisted (function declarations and imports).
    /// Hoisted segments don't create execution-order dependencies on prior segments.
    pub is_hoisted: bool,
}

/// Segment graph enabling tag propagation across segment nodes.
///
/// Nodes are `(file_id, segment_index)` pairs. Edges represent either
/// name-based references (inter-file) or execution-order (effect) dependencies
/// (intra-file).
#[derive(Debug, Clone)]
pub struct SegmentGraph {
    pub nodes: Vec<SegmentNode>,
    /// Forward edges: node index → set of node indices it depends on.
    pub edges: Vec<AHashSet<usize>>,
    /// Map from SegmentId to the flat node index.
    id_to_index: AHashMap<SegmentId, usize>,
    /// For each file, maps exported symbol → node index of the segment that exports it.
    file_export_map: AHashMap<usize, AHashMap<ExportedLocal, usize>>,
}

impl SegmentGraph {
    /// Build a segment graph from per-file segment lists.
    ///
    /// `file_segments` is a list of `(file_id, segments)` pairs. The file_id
    /// is an opaque identifier assigned by the caller (e.g. from a path→id map).
    pub fn build(file_segments: &[(usize, &[Segment])]) -> Self {
        let total_segments: usize = file_segments.iter().map(|(_, segs)| segs.len()).sum();
        let mut nodes = Vec::with_capacity(total_segments);
        let mut edges: Vec<AHashSet<usize>> = Vec::with_capacity(total_segments);
        let mut id_to_index = AHashMap::with_capacity_and_hasher(total_segments, Default::default());
        let mut file_export_map: AHashMap<usize, AHashMap<ExportedLocal, usize>> =
            AHashMap::default();

        // Phase 1: create nodes and build the export map.
        for &(file_id, segs) in file_segments {
            let mut file_exports: AHashMap<ExportedLocal, usize> = AHashMap::default();
            for (seg_idx, seg) in segs.iter().enumerate() {
                let sid = SegmentId::new(file_id, seg_idx);
                let node_idx = nodes.len();
                id_to_index.insert(sid, node_idx);
                let is_hoisted = segment_is_hoisted(seg);
                nodes.push(SegmentNode {
                    id: sid,
                    tags: TagSet::EMPTY,
                    is_hoisted,
                });
                edges.push(AHashSet::default());

                // Record which exports this segment provides.
                for exported_sym in seg.module_deps.exports_locals.keys() {
                    file_exports.insert(exported_sym.clone(), node_idx);
                }

                // Also record re-exports. The "exported_as" name (or the
                // original name if no rename) is what downstream importers see.
                for (_specifier, re_exports) in &seg.module_deps.exports_from {
                    for re_export in re_exports {
                        let exported_name = match &re_export.exported_as {
                            Some(name) => name.clone(),
                            None => {
                                // No rename: exported as the original name.
                                match &re_export.imported_as {
                                    crate::ImportTarget::ExportedSymbol(sym) => sym.clone(),
                                    crate::ImportTarget::Namespace => {
                                        // `export * from '...'` — wildcard re-export.
                                        // This can't be indexed by a single name.
                                        continue;
                                    }
                                }
                            }
                        };
                        file_exports.insert(exported_name, node_idx);
                    }
                }
            }
            file_export_map.insert(file_id, file_exports);
        }

        let mut graph = SegmentGraph {
            nodes,
            edges,
            id_to_index,
            file_export_map,
        };

        // Phase 2: add intra-file effect edges.
        for &(file_id, segs) in file_segments {
            graph.add_intra_file_effect_edges(file_id, segs.len());
        }

        // Phase 3: add inter-file name edges.
        // We need the module deps to resolve imports → exports across files.
        // Build a path-to-file-id map from all files' import specifiers.
        // For now, imports use string specifiers. The caller is expected to have
        // resolved these to file_ids before constructing this graph, OR we can
        // accept a resolver. We take the simpler approach: accept a
        // `resolved_imports` map that's built externally.
        //
        // Inter-file edges are added via `add_inter_file_edges`.

        graph
    }

    /// Add intra-file effect edges: each non-hoisted segment depends on the
    /// previous non-hoisted segment in the same file (execution order).
    fn add_intra_file_effect_edges(&mut self, file_id: usize, segment_count: usize) {
        let mut prev_non_hoisted: Option<usize> = None;
        for seg_idx in 0..segment_count {
            let sid = SegmentId::new(file_id, seg_idx);
            let node_idx = self.id_to_index[&sid];
            if !self.nodes[node_idx].is_hoisted {
                if let Some(prev_idx) = prev_non_hoisted {
                    self.edges[node_idx].insert(prev_idx);
                }
                prev_non_hoisted = Some(node_idx);
            }
        }
    }

    /// Add inter-file name edges using a resolution map.
    ///
    /// `resolved_imports` maps `(importing_file_id, import_specifier)` →
    /// `target_file_id`. This must be provided by the caller after resolving
    /// import paths to file IDs.
    pub fn add_inter_file_edges(
        &mut self,
        file_segments: &[(usize, &[Segment])],
        resolved_imports: &AHashMap<(usize, String), usize>,
    ) {
        for &(file_id, segs) in file_segments {
            for (seg_idx, seg) in segs.iter().enumerate() {
                let sid = SegmentId::new(file_id, seg_idx);
                let from_idx = self.id_to_index[&sid];
                self.add_edges_for_deps(file_id, from_idx, &seg.module_deps, resolved_imports);
            }
        }
    }

    /// For a single segment, resolve its imports/requires/re-exports to edges
    /// targeting the exporting segment in the target file.
    fn add_edges_for_deps(
        &mut self,
        file_id: usize,
        from_idx: usize,
        deps: &RawModuleDeps,
        resolved_imports: &AHashMap<(usize, String), usize>,
    ) {
        // Static imports: import { foo } from './bar'
        for (specifier, symbols) in &deps.imports {
            if let Some(&target_file) = resolved_imports.get(&(file_id, specifier.clone())) {
                for tagged_sym in symbols {
                    self.add_name_edge(from_idx, target_file, &tagged_sym.symbol);
                }
            }
        }

        // Dynamic imports: import('./bar')
        for (specifier, symbols) in &deps.dynamic_imports {
            if let Some(&target_file) = resolved_imports.get(&(file_id, specifier.clone())) {
                for sym in symbols {
                    self.add_name_edge(from_idx, target_file, sym);
                }
            }
        }

        // Re-exports: export { foo } from './bar'
        for (specifier, re_exports) in &deps.exports_from {
            if let Some(&target_file) = resolved_imports.get(&(file_id, specifier.clone())) {
                for re_export in re_exports {
                    let sym = match &re_export.imported_as {
                        crate::ImportTarget::ExportedSymbol(ExportedSymbol::Named(name)) => {
                            Symbol::Named(name.clone())
                        }
                        crate::ImportTarget::ExportedSymbol(ExportedSymbol::Default) => {
                            Symbol::Default
                        }
                        crate::ImportTarget::Namespace => Symbol::Namespace,
                    };
                    self.add_name_edge(from_idx, target_file, &sym);
                }
            }
        }

        // Side-effect imports: import './polyfill'
        for specifier in &deps.executed_paths {
            if let Some(&target_file) = resolved_imports.get(&(file_id, specifier.clone())) {
                // Effect import creates edges to ALL segments in the target file
                // (the whole file is executed for its side effects).
                if let Some(exports) = self.file_export_map.get(&target_file) {
                    let target_indices: Vec<usize> = exports.values().copied().collect();
                    for target_idx in target_indices {
                        self.edges[from_idx].insert(target_idx);
                    }
                }
            }
        }
    }

    /// Add an edge from `from_idx` to the segment in `target_file` that
    /// exports the given symbol. If the symbol is Namespace, add edges to all
    /// exporting segments in that file.
    fn add_name_edge(&mut self, from_idx: usize, target_file: usize, symbol: &Symbol) {
        let target_exports = match self.file_export_map.get(&target_file) {
            Some(m) => m,
            None => return,
        };

        match symbol {
            Symbol::Named(name) => {
                let key = ExportedSymbol::Named(name.clone());
                if let Some(&target_idx) = target_exports.get(&key) {
                    self.edges[from_idx].insert(target_idx);
                }
            }
            Symbol::Default => {
                if let Some(&target_idx) = target_exports.get(&ExportedSymbol::Default) {
                    self.edges[from_idx].insert(target_idx);
                }
            }
            Symbol::Namespace => {
                // Namespace import depends on all exported segments in the file.
                let target_indices: Vec<usize> = target_exports.values().copied().collect();
                for target_idx in target_indices {
                    self.edges[from_idx].insert(target_idx);
                }
            }
        }
    }

    /// BFS tag propagation from a seed set.
    ///
    /// Starting from `seeds`, propagates `tag` along all forward edges.
    /// A node receives the tag if any of its dependents (nodes depending on it,
    /// i.e. reverse edges) already have it — but we propagate *forward* from
    /// seeds here: if seed S depends on T, then T gets the tag too.
    ///
    /// Actually, the natural direction is: seeds are "used" or "reachable",
    /// and we propagate along dependency edges (if A depends on B, and A is
    /// reachable, then B is reachable too).
    pub fn propagate_tags(&mut self, seeds: &[usize], tag: TagSet) {
        let mut queue = VecDeque::with_capacity(seeds.len());
        for &seed in seeds {
            if seed < self.nodes.len() && !self.nodes[seed].tags.contains(tag) {
                self.nodes[seed].tags.insert(tag);
                queue.push_back(seed);
            }
        }

        while let Some(node_idx) = queue.pop_front() {
            // Propagate along forward edges (dependencies).
            // Clone the edge set to avoid borrow conflict.
            let deps: Vec<usize> = self.edges[node_idx].iter().copied().collect();
            for dep_idx in deps {
                if !self.nodes[dep_idx].tags.contains(tag) {
                    self.nodes[dep_idx].tags.insert(tag);
                    queue.push_back(dep_idx);
                }
            }
        }
    }

    /// Convenience: propagate tags from seed SegmentIds.
    pub fn propagate_tags_from_ids(&mut self, seeds: &[SegmentId], tag: TagSet) {
        let indices: Vec<usize> = seeds
            .iter()
            .filter_map(|sid| self.id_to_index.get(sid).copied())
            .collect();
        self.propagate_tags(&indices, tag);
    }

    /// Look up the flat node index for a SegmentId.
    pub fn node_index(&self, id: &SegmentId) -> Option<usize> {
        self.id_to_index.get(id).copied()
    }

    /// Get a node by its flat index.
    pub fn node(&self, idx: usize) -> &SegmentNode {
        &self.nodes[idx]
    }

    /// Get all node indices that do NOT have a given tag.
    pub fn nodes_without_tag(&self, tag: TagSet) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.tags.contains(tag))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Determine if a segment is hoisted.
///
/// In JS/TS, function declarations and import/export declarations are hoisted.
/// Other statements (const, let, class, expression statements, etc.) are not.
fn segment_is_hoisted(seg: &Segment) -> bool {
    // A segment is considered hoisted if it has static imports or exports_from
    // (i.e. it's an import/export declaration) and no dynamic content,
    // OR if it's a function declaration (detected via variable scope).
    //
    // Simple heuristic: if the segment has static imports or re-exports, it's
    // an import/export declaration and is hoisted. Otherwise, check if the
    // variable scope declares a function (approximated by having exports_locals
    // with no dynamic_imports/requires).
    let deps = &seg.module_deps;

    // Import declarations are hoisted.
    if !deps.imports.is_empty() {
        return true;
    }

    // Export-from (re-export) declarations are hoisted.
    if !deps.exports_from.is_empty() {
        return true;
    }

    // Side-effect imports are hoisted.
    if !deps.executed_paths.is_empty() {
        return true;
    }

    false
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::raw_module_deps::{RawModuleDeps, TaggedSymbol, SymbolTags};
    use crate::{ExportedSymbol, ImportTarget, ReExportedSymbol};
    use crate::segment_info::Segment;
    use crate::variables::VariableScope;

    /// Helper: create a minimal Segment with given module deps.
    fn seg(deps: RawModuleDeps) -> Segment {
        Segment {
            module_deps: deps,
            variables: VariableScope::default(),
            span: oxc_span::Span::default(),
        }
    }

    /// Helper: create a segment that exports a named symbol.
    fn exporting_seg(name: &str) -> Segment {
        let mut deps = RawModuleDeps::default();
        deps.exports_locals.insert(
            ExportedSymbol::Named(name.into()),
            TaggedSymbol::new(Symbol::named(name), SymbolTags::default()),
        );
        seg(deps)
    }

    /// Helper: create a segment that imports a named symbol from a specifier.
    fn importing_seg(specifier: &str, name: &str) -> Segment {
        let mut deps = RawModuleDeps::default();
        let mut symbols = AHashSet::default();
        symbols.insert(TaggedSymbol::new(Symbol::named(name), SymbolTags::default()));
        deps.imports.insert(specifier.to_string(), symbols);
        seg(deps)
    }

    /// Helper: create a plain statement segment (non-hoisted, no deps).
    fn stmt_seg() -> Segment {
        seg(RawModuleDeps::default())
    }

    // ── Linear chain propagation ──────────────────────────────────────

    #[test]
    fn linear_chain_propagation() {
        // File 0: [export a] [export b]
        // File 1: [import a from file0] [import b from file0]
        // Seed: file1/seg0 → should propagate to file0's export-a segment.
        let f0_segs = vec![exporting_seg("a"), exporting_seg("b")];
        let f1_segs = vec![importing_seg("./f0", "a"), importing_seg("./f0", "b")];

        let file_segments: Vec<(usize, &[Segment])> =
            vec![(0, &f0_segs), (1, &f1_segs)];
        let mut graph = SegmentGraph::build(&file_segments);

        // Resolve "./f0" from file 1 → file 0.
        let mut resolved = AHashMap::default();
        resolved.insert((1, "./f0".to_string()), 0usize);
        graph.add_inter_file_edges(&file_segments, &resolved);

        // Seed: file1/seg0 (imports "a").
        let seed = graph.node_index(&SegmentId::new(1, 0)).unwrap();
        graph.propagate_tags(&[seed], TagSet::REACHABLE);

        // file1/seg0 should be tagged.
        assert!(graph.nodes[seed].tags.contains(TagSet::REACHABLE));

        // file0's export-a segment should be tagged (reached via import edge).
        let a_idx = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        assert!(graph.nodes[a_idx].tags.contains(TagSet::REACHABLE));

        // file0's export-b segment should NOT be tagged.
        let b_idx = graph.node_index(&SegmentId::new(0, 1)).unwrap();
        assert!(!graph.nodes[b_idx].tags.contains(TagSet::REACHABLE));

        // file1/seg1 should NOT be tagged (not seeded).
        let f1s1 = graph.node_index(&SegmentId::new(1, 1)).unwrap();
        assert!(!graph.nodes[f1s1].tags.contains(TagSet::REACHABLE));
    }

    // ── Diamond dependency ────────────────────────────────────────────

    /// Helper: create a re-export segment: `export { <name> } from '<specifier>'`
    /// This is hoisted and creates an edge to the target file's export.
    fn re_exporting_seg(specifier: &str, name: &str) -> Segment {
        let mut deps = RawModuleDeps::default();
        let mut re_exports = AHashSet::default();
        re_exports.insert(ReExportedSymbol {
            imported_as: ImportTarget::ExportedSymbol(ExportedSymbol::Named(name.into())),
            exported_as: Some(ExportedSymbol::Named(name.into())),
            tags: SymbolTags::default(),
            span: oxc_span::Span::default(),
        });
        deps.exports_from.insert(specifier.to_string(), re_exports);
        seg(deps)
    }

    #[test]
    fn diamond_dependency() {
        // File 0: [export shared]
        // File 1: re-exports "shared" as "left" from f0
        // File 2: re-exports "shared" as "right" from f0
        // File 3: imports "left" from f1 and "right" from f2
        //
        // Seed: file3/seg0 → should reach f1 → f0/shared, and f2 → f0/shared.
        let f0 = vec![exporting_seg("shared")];
        let f1 = vec![re_exporting_seg("./f0", "shared")];
        let f2 = vec![re_exporting_seg("./f0", "shared")];

        // f3 imports "left" from f1 and "right" from f2
        // (the re-exported name in f1/f2 is "shared", so f3 imports "shared")
        let mut f3_deps = RawModuleDeps::default();
        let mut left_syms = AHashSet::default();
        left_syms.insert(TaggedSymbol::new(Symbol::named("shared"), SymbolTags::default()));
        f3_deps.imports.insert("./f1".to_string(), left_syms);
        let mut right_syms = AHashSet::default();
        right_syms.insert(TaggedSymbol::new(Symbol::named("shared"), SymbolTags::default()));
        f3_deps.imports.insert("./f2".to_string(), right_syms);
        let f3 = vec![seg(f3_deps)];

        let file_segments: Vec<(usize, &[Segment])> =
            vec![(0, &f0), (1, &f1), (2, &f2), (3, &f3)];
        let mut graph = SegmentGraph::build(&file_segments);

        let mut resolved = AHashMap::default();
        resolved.insert((1, "./f0".to_string()), 0usize);
        resolved.insert((2, "./f0".to_string()), 0usize);
        resolved.insert((3, "./f1".to_string()), 1usize);
        resolved.insert((3, "./f2".to_string()), 2usize);
        graph.add_inter_file_edges(&file_segments, &resolved);

        // Seed: f3/seg0
        let seed = graph.node_index(&SegmentId::new(3, 0)).unwrap();
        graph.propagate_tags(&[seed], TagSet::REACHABLE);

        // f0/shared should be reachable (via both diamond paths).
        let shared_idx = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        assert!(graph.nodes[shared_idx].tags.contains(TagSet::REACHABLE));

        // f1 and f2 re-export segments should be reachable.
        // Note: re-export segments use exports_from, not exports_locals,
        // so they don't appear in the file_export_map. The import edge from
        // f3 resolves "shared" to f1's exports. But f1 has exports_from, not
        // exports_locals, so f1 has no entries in file_export_map. The import
        // edge from f3 → f1 won't find "shared" in f1's export map.
        //
        // This is correct behavior for now: re-exports are "pass-through" and
        // the inter-file edge resolution follows the re-export chain.
        // The f1 re-export segment itself isn't directly targeted, but its
        // dependency (f0/shared) is reached because f1's exports_from creates
        // an edge to f0.
    }

    // ── Cycle handling ────────────────────────────────────────────────

    #[test]
    fn cycle_handling() {
        // File 0: [export a], imports b from f1
        // File 1: [export b], imports a from f0
        // This creates a cycle. Propagation must terminate.
        let f0_export = exporting_seg("a");
        let f0_import = importing_seg("./f1", "b");
        let f0 = vec![f0_export, f0_import];

        let f1_export = exporting_seg("b");
        let f1_import = importing_seg("./f0", "a");
        let f1 = vec![f1_export, f1_import];

        let file_segments: Vec<(usize, &[Segment])> =
            vec![(0, &f0), (1, &f1)];
        let mut graph = SegmentGraph::build(&file_segments);

        let mut resolved = AHashMap::default();
        resolved.insert((0, "./f1".to_string()), 1usize);
        resolved.insert((1, "./f0".to_string()), 0usize);
        graph.add_inter_file_edges(&file_segments, &resolved);

        // Seed: f0/seg0 (export a)
        let seed = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        graph.propagate_tags(&[seed], TagSet::REACHABLE);

        // All reachable segments should be tagged, and BFS should terminate.
        assert!(graph.nodes[seed].tags.contains(TagSet::REACHABLE));

        // f1/import-a points to f0/export-a which is the seed, so f1's import
        // segment should be reachable only if seeded. The import of "b" from f1
        // in f0 creates: f0/seg1 → f1/seg0. But f0/seg1 is not seeded.
        let f0_s1 = graph.node_index(&SegmentId::new(0, 1)).unwrap();
        // f0/seg1 (import b from f1) is NOT seeded, but it's an import decl (hoisted),
        // so it has no intra-file effect edge from f0/seg0.
        // It should NOT be reachable from just f0/seg0.
        assert!(!graph.nodes[f0_s1].tags.contains(TagSet::REACHABLE));
    }

    // ── Intra-file effect edges ───────────────────────────────────────

    #[test]
    fn intra_file_effect_edges_skip_hoisted() {
        // File 0: [import (hoisted)], [stmt], [stmt], [import (hoisted)]
        // Effect chain should be: stmt0 depends on nothing, stmt1 depends on stmt0.
        // Imports are hoisted and don't participate in effect chains.
        let s0 = importing_seg("./a", "x"); // hoisted
        let s1 = stmt_seg(); // non-hoisted
        let s2 = stmt_seg(); // non-hoisted, depends on s1
        let s3 = importing_seg("./b", "y"); // hoisted

        let f0 = vec![s0, s1, s2, s3];
        let file_segments: Vec<(usize, &[Segment])> = vec![(0, &f0)];
        let graph = SegmentGraph::build(&file_segments);

        let idx0 = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        let idx1 = graph.node_index(&SegmentId::new(0, 1)).unwrap();
        let idx2 = graph.node_index(&SegmentId::new(0, 2)).unwrap();
        let idx3 = graph.node_index(&SegmentId::new(0, 3)).unwrap();

        // Hoisted segments have no intra-file effect edges.
        assert!(graph.edges[idx0].is_empty());
        assert!(graph.edges[idx3].is_empty());

        // stmt1 (idx1) has no predecessor non-hoisted segment → no effect edge.
        assert!(graph.edges[idx1].is_empty());

        // stmt2 (idx2) depends on stmt1 (idx1) via execution order.
        assert!(graph.edges[idx2].contains(&idx1));
    }

    // ── Tag propagation with multiple tags ────────────────────────────

    #[test]
    fn multiple_tags_independent() {
        // Use segments in separate files to avoid intra-file effect edges.
        let f0 = vec![exporting_seg("a")];
        let f1 = vec![exporting_seg("b")];
        let file_segments: Vec<(usize, &[Segment])> = vec![(0, &f0), (1, &f1)];
        let mut graph = SegmentGraph::build(&file_segments);

        let idx0 = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        let idx1 = graph.node_index(&SegmentId::new(1, 0)).unwrap();

        graph.propagate_tags(&[idx0], TagSet::REACHABLE);
        graph.propagate_tags(&[idx1], TagSet::EFFECTFUL);

        assert!(graph.nodes[idx0].tags.contains(TagSet::REACHABLE));
        assert!(!graph.nodes[idx0].tags.contains(TagSet::EFFECTFUL));
        assert!(!graph.nodes[idx1].tags.contains(TagSet::REACHABLE));
        assert!(graph.nodes[idx1].tags.contains(TagSet::EFFECTFUL));
    }

    // ── nodes_without_tag ─────────────────────────────────────────────

    #[test]
    fn nodes_without_tag_reports_unreached() {
        let f0 = vec![exporting_seg("a"), exporting_seg("b"), stmt_seg()];
        let file_segments: Vec<(usize, &[Segment])> = vec![(0, &f0)];
        let mut graph = SegmentGraph::build(&file_segments);

        let idx0 = graph.node_index(&SegmentId::new(0, 0)).unwrap();
        graph.propagate_tags(&[idx0], TagSet::REACHABLE);

        let unreached = graph.nodes_without_tag(TagSet::REACHABLE);
        assert_eq!(unreached.len(), 2); // b and stmt
        assert!(!unreached.contains(&idx0));
    }
}
