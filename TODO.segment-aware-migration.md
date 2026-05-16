# Segment-Aware Migration

High level goal: extend unused-export tracking from file-level granularity to per-statement (segment) granularity. This enables identifying individual unused statements rather than just unused files, by tracking which names each segment references that aren't shadowed locally. The `ast_segmenter` crate defines the new type system; `unused_finder` will adopt it.

Decision: `unused_finder` adopts `ast_segmenter`'s types (`Symbol`, `TaggedSymbol`, `RawModuleDeps`, `ExportedSymbol`, etc.) rather than maintaining its own parallel hierarchy.

## Make `ast_segmenter` compile

Fix the 14 compilation errors so the crate builds cleanly. This is purely mechanical — no new features.

Key issues:
- `visitor.rs`: syntax errors in `module_item_to_segment` (destructuring into nonexistent vars), undefined module path `crate::import_require` (should be `crate::visitors::import_require_expr`), references to undefined `stmt` in the `ModuleDecl` arm, call to nonexistent `statement_to_segment`
- `visitor.rs`: `Dependencies2D` uses `roaring::RoaringBitmap` which isn't in Cargo.toml — either add `roaring` or replace with `Vec<bool>` for now
- `raw_module_deps.rs`: `TryInto<ReExportedSymbol> for ExportBinding` has malformed syntax (`Error = ...` instead of `type Error = ...`)
- `raw_module_deps.rs`: `ExportedLocal` type referenced in `RawModuleDeps::exports_locals` but never defined
- `visitors/import_export_statement.rs`: `From<ExportsVisitor<T>> for RawImportExportInfo` references a type that doesn't exist in this crate — convert to `From<ExportsVisitor<T>> for RawModuleDeps`
- `visitors/import_export_statement.rs`: `ExportsVisitor` struct fields still use old unused_finder field names — align with `RawModuleDeps` fields
- `visitors/import_export_statement.rs`: `visit_named_export` calls `.try_as_re_export()` which doesn't exist on `ExportBinding`

- [x] Fix `TryInto` impl syntax and add `ExportedLocal` type in `raw_module_deps.rs`
- [x] Add `roaring` to Cargo.toml
- [x] Fix `visitor.rs` imports, variable references, and function names
- [x] Rewrite `ExportsVisitor` to populate `RawModuleDeps` directly (remove `RawImportExportInfo` reference)
- [x] Fix `visit_named_export` to use the correct conversion method on `ExportBinding`
- [x] Run `cargo check -p ast-segmenter` — zero errors

## Complete the segment orchestrator

Implement `segment_module()` in `visitor.rs` so it produces a `Vec<RawSegment>` — one per top-level `ModuleItem`. Each segment combines variable scope analysis (`ast_name_tracker::find_names`) with module dependency extraction (`ExportsVisitor` / `ImportsAndRequires`).

```rust
// Target public API in ast_segmenter
pub fn segment_file(
    logger: &impl SrcFileLogger,
    module: &Module,
    comments: &SingleThreadedComments,
) -> Vec<RawSegment>;
```

Each `RawSegment` = `{ module_deps: RawModuleDeps, variables: VariableScope }`.

For `ModuleItem::Stmt`: run `find_names` + `find_imports_and_requires` on the statement.
For `ModuleItem::ModuleDecl`: run `find_names` + `ExportsVisitor` on the declaration.

- [x] Implement `module_item_to_segment` for `Stmt` variants (combine name tracker + dynamic import finder)
- [x] Implement `module_item_to_segment` for `ModuleDecl` variants (combine name tracker + exports visitor)
- [x] Implement `segment_file` as the public entry point that iterates `module.body`
- [x] Add snapshot tests: simple file with mixed imports/exports/statements → verify segment count and contents
- [x] Add snapshot tests: file with side-effect imports, re-exports, dynamic imports

## Build the segment graph

Create a `SegmentGraph` that enables tagging and propagating properties (effectful, reachable-from-root) across segment nodes. Nodes are `(file_id, segment_index)` pairs. Edges represent either name-based references or execution-order (effect) dependencies.

```rust
pub struct SegmentId {
    pub file_id: usize,
    pub segment_idx: usize,
}

pub struct SegmentGraph {
    nodes: Vec<SegmentNode>,
    /// Forward edges: node → set of nodes it depends on
    edges: Vec<AHashSet<usize>>,
}
```

Tag propagation: BFS/DFS from seed nodes, propagating tags along edges. Same pattern as existing `Graph::traverse_bfs` but at segment granularity.

- [ ] Define `SegmentId`, `SegmentNode`, `SegmentGraph` types
- [ ] Implement intra-file effect edges (each segment depends on prior non-hoisted segments in the same file)
- [ ] Implement inter-file name edges (segment importing symbol X from file Y → edge to the segment in Y that exports X)
- [ ] Implement `propagate_tags` — BFS from a seed set, propagating a bitflag tag along edges
- [ ] Add unit tests: linear chain propagation, diamond dependency, cycle handling

## Wire `ast_segmenter` into `unused_finder`

Replace `unused_finder`'s internal parse pipeline with `ast_segmenter`'s `segment_file`. Adopt `ast_segmenter`'s types throughout.

Integration points:
- `parse/exports_visitor_runner.rs` → call `ast_segmenter::segment_file` instead of local `get_file_import_export_info`
- `parse/data.rs` → remove `RawImportExportInfo`, `ExportedSymbol`, `ReExportedSymbol`; re-export from `ast_segmenter`
- `graph.rs` → `GraphFile` gains `segments: Vec<RawSegment>`, symbol lookups index into segments
- `walked_file.rs` → `ResolvedSourceFile` carries segment-level data

- [ ] Add `ast-segmenter` as a dependency of `unused_finder`
- [ ] Replace `parse/data.rs` types with re-exports from `ast_segmenter::raw_module_deps` and `ast_segmenter::ExportedSymbol`
- [ ] Replace `get_file_import_export_info` call site with `segment_file`
- [ ] Adapt `ResolvedSourceFile` to carry `Vec<RawSegment>` (or a resolved equivalent)
- [ ] Adapt `GraphFile` to index symbols back to their source segment
- [ ] Ensure existing tests pass (file-level behavior preserved as a degenerate case of segment-level)

## Segment-level unused reporting

Extend the graph traversal and reporting to identify unused segments (statements that are neither reachable from an entry point nor transitively depended upon by reachable segments).

- [ ] Extend `traverse_bfs` (or use `SegmentGraph::propagate_tags`) to mark segments as reachable
- [ ] Add `SegmentReport` type — identifies unused segments by file + span/line range
- [ ] Update `UnusedFinderReport` to include per-segment results alongside existing per-file results
- [ ] Add integration test: file with one used and one unused top-level declaration → only the unused one reported
- [ ] Add integration test: side-effect statement keeps its transitive dependencies alive
