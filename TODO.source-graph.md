# SourceGraph + TagGraph

High level goal: replace the current `Graph`/`GraphFile` in `unused_finder` with two
decoupled structures — `SourceGraph` (owns segments, resolves names to segments) and
`TagGraph` (tracks usage tags against segment keys). This enables segment-level unused
detection and supports both upward (toward roots) and downward (toward leaves) tag propagation.

Design decisions:
- `SegmentKey` is a Copy struct of two `u32`s (`file_id`, `segment_idx`)
- Intra-file resolution is hoisting-aware (uses existing `VariableScope` + `HoistingLevel`)
- Re-export chains are followed lazily with a cache
- `TagGraph` tracks tags per-segment only; callers derive file-level tags
- This replaces `traverse_bfs` entirely (no coexistence period)
- Upward propagation: if segment B is tagged, importers of B get tagged too
- Edges carry `is_type_only` metadata; `TagGraph` decides whether to follow them
- `SourceGraph` is designed for single-file incremental patching
- `SourceGraph` owns its segment data (cloned from parsed data)
- Cross-file resolution returns `Vec<SegmentKey>` (star re-exports fan out)
- Both types live in new crate(s)

References:
- Current graph: `crates/unused_finder/src/graph.rs` (`Graph`, `GraphFile`, `Edge`)
- Segment type: `crates/ast_segmenter/src/segment_info.rs` (`Segment`)
- Name tracking: `crates/ast_name_tracker/src/visitor.rs` (`VariableScope`, `HoistingLevel`)
- Resolved imports: `crates/unused_finder/src/parse/data.rs` (`ResolvedImportExportInfo`, `ExportedSymbolMetadata`)
- Call sites: `crates/unused_finder/src/unused_finder.rs` lines 529–569 (3× `traverse_bfs`)

## Create `source_graph` crate with core types

Create `crates/source_graph/` with `SegmentKey`, edge types, and the `SourceGraph` skeleton.

```rust
// segment_key.rs
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct SegmentKey {
    pub file_id: u32,
    pub segment_idx: u32,
}

// edge.rs
pub struct SegmentEdge {
    pub from: SegmentKey,
    pub to: SegmentKey,
    pub is_type_only: bool,
}
```

`SourceGraphFile` holds per-file owned data and pre-built indexes:

```rust
struct SourceGraphFile {
    path: PathBuf,
    segments: Vec<Segment>,
    // exported symbol → segment index that declares the export
    symbol_to_segment: AHashMap<ExportedSymbol, u32>,
    // local name → Vec<(segment_idx, HoistingLevel)> for intra-file lookup
    name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>>,
    import_export_info: ResolvedImportExportInfo,
}
```

- [x] Create crate at `crates/source_graph` with `Cargo.toml`, add to workspace members
- [x] Define `SegmentKey` in `segment_key.rs`
- [x] Define `SegmentEdge` in `edge.rs`
- [x] Define `SourceGraphFile` (private) and `SourceGraph` (public) structs in `lib.rs`
- [x] Implement `SourceGraph::new()` from an iterator of `SourceFileInput` — build `path_to_id`, `files`, `symbol_to_segment` and `name_to_declaring_segments` indexes per file
- [x] Add unit tests for construction from a simple `SourceFileInput`

## Implement `resolve_symbol_in_file`

Resolve a local name to its declaring segment within a file, respecting JS hoisting semantics.

Hoisting rules (from `ast_name_tracker::HoistingLevel`):
- `ImportHoisting` → visible to all segments in the file (type 1+4 hoisting)
- `FunctionHoisting` → visible to all segments in the file (type 1 hoisting)
- `LetConstHoisting` → visible only to segments at the same or later index

```rust
impl SourceGraph {
    /// Resolve a name reference to the segment that declares it within the same file.
    /// Returns None if the name is not declared in any segment of this file.
    pub fn resolve_symbol_in_file(
        &self,
        file_id: u32,
        name: &str,
        referencing_segment_idx: u32,
    ) -> Option<SegmentKey>;
}
```

When multiple declarations exist for the same name (shadowing), prefer the latest
declaration at or before `referencing_segment_idx` for `LetConstHoisting`, or the
first declaration for hoisted levels.

- [x] Implement `resolve_symbol_in_file` using `name_to_declaring_segments` index
- [x] Test: import-hoisted name resolved from a later segment
- [x] Test: function-hoisted name resolved from an earlier segment
- [x] Test: let/const name NOT resolved from an earlier segment
- [x] Test: name not found returns `None`

## Implement `resolve_import_across_files`

Resolve an inter-file import `(file_id, ExportedSymbol)` to the segment(s) in the
target file that export that symbol. For re-exports (`export { x } from './b'`),
lazily follow the chain to the originating segment and cache the result.

```rust
impl SourceGraph {
    /// Resolve an import of `symbol` from `target_file_id` to the segment(s)
    /// that ultimately declare/export it. Follows re-export chains lazily,
    /// caching results. Returns empty Vec if the symbol is not found.
    pub fn resolve_import_across_files(
        &self,
        target_file_id: u32,
        symbol: &ExportedSymbol,
    ) -> Vec<SegmentKey>;
}
```

For star re-exports (`export * from`), fan out: return a `SegmentKey` for each
matching exported symbol in the re-exported file.

The re-export cache is interior-mutated (`RefCell<AHashMap<(u32, ExportedSymbol), Vec<SegmentKey>>>`)
so this method can take `&self`.

- [x] Implement direct export lookup (symbol found in `symbol_to_segment` of target file)
- [x] Implement re-export chain following with cycle detection
- [x] Implement star re-export fan-out
- [x] Add `RefCell`-based cache, invalidated by `patch_file`
- [x] Test: direct export resolves to single `SegmentKey`
- [x] Test: re-export chain A → B → C resolves to segment in C
- [x] Test: star re-export fans out to multiple `SegmentKey`s
- [x] Test: cycle in re-exports terminates without panic

## Implement iterators and `patch_file`

```rust
impl SourceGraph {
    pub fn iter_file_segments(&self, path: &Path)
        -> Option<impl Iterator<Item = (SegmentKey, &Segment)>>;

    pub fn iter_segments(&self)
        -> impl Iterator<Item = (SegmentKey, &Segment)>;

    /// Replace a single file's data in-place. Rebuilds that file's indexes
    /// and invalidates cached re-export resolutions that touched this file.
    pub fn patch_file(&mut self, path: &Path, file: &ResolvedSourceFile);
}
```

- [x] Implement `iter_file_segments`
- [x] Implement `iter_segments`
- [x] Implement `patch_file` — replace file entry, rebuild its indexes, clear relevant cache entries
- [x] Test: `patch_file` updates resolution results
- [x] Test: iterators yield expected `(SegmentKey, &Segment)` pairs

## Create `tag_graph` crate

`TagGraph` stores per-segment tags and implements bidirectional BFS propagation.
It borrows a `SourceGraph` for edge resolution but owns the tag state independently.

```rust
pub struct TagGraph {
    tags: AHashMap<SegmentKey, UsedTag>,
}

impl TagGraph {
    pub fn get_tag(&self, key: SegmentKey) -> UsedTag;

    /// Derived: union of all segment tags for segments belonging to `file_id`.
    pub fn file_tag(&self, source: &SourceGraph, file_id: u32) -> UsedTag;
}
```

- [x] Create crate at `crates/tag_graph` with `Cargo.toml`, add to workspace members
- [x] Define `TagGraph` struct with `tags: AHashMap<SegmentKey, UsedTag>`
- [x] Implement `get_tag` and `file_tag`
- [x] Add basic unit tests for tag storage and file-level derivation

## Implement `propagate_tags_to_used` (downward)

BFS from root segments downward along import edges. This replaces `Graph::traverse_bfs`.

For each segment in the frontier:
1. Tag the segment
2. Find inter-file imports via `ResolvedImportExportInfo::iter_imported_symbols_meta`
3. Resolve each import to target `SegmentKey`(s) via `SourceGraph::resolve_import_across_files`
4. Skip edges where `is_type_only` is true (configurable per propagation)
5. Find intra-file dependencies via `VariableScope::escaped_symbols` → `resolve_symbol_in_file`
6. Add unvisited targets to the next frontier

```rust
impl TagGraph {
    pub fn propagate_tags_to_used(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    );
}
```

- [ ] Implement downward BFS with inter-file edge resolution
- [ ] Implement intra-file edge resolution via escaped symbols + hoisting
- [ ] Implement `is_type_only` edge filtering controlled by `follow_type_only` parameter
- [ ] Test: linear import chain propagates tag to all segments
- [ ] Test: diamond dependency — tag propagates through both paths
- [ ] Test: type-only edge skipped when `follow_type_only` is false
- [ ] Test: cycle terminates cleanly
- [ ] Test: intra-file escaped symbol creates edge to declaring segment

## Implement `propagate_tags_to_users` (upward)

BFS from tagged segments upward toward roots — find all segments that transitively
depend on the tagged set. This requires reverse edges (who imports me?).

Build reverse edges lazily or during construction: for each import edge A→B,
record B→A in a reverse adjacency structure.

```rust
impl TagGraph {
    pub fn propagate_tags_to_users(
        &mut self,
        source: &SourceGraph,
        seeds: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    );
}
```

- [ ] Build or lazily compute reverse edge index from `SourceGraph`
- [ ] Implement upward BFS using reverse edges
- [ ] Test: tagging a leaf propagates upward to its importer
- [ ] Test: tagging a mid-graph node propagates to all transitive importers
- [ ] Test: upward propagation does not traverse downward

## Wire `SourceGraph` + `TagGraph` into `unused_finder`

Replace the 3 call sites in `unused_finder.rs` (lines 529–569) that use
`Graph::from_source_files` + `traverse_bfs` with the new types.

Call sites:
- `Graph::from_source_files` → `SourceGraph::new`
- `traverse_bfs(..., UsedTag::FROM_ENTRY)` → `propagate_tags_to_used(..., FROM_ENTRY, false)`
- `traverse_bfs(..., UsedTag::FROM_IGNORED)` → `propagate_tags_to_used(..., FROM_IGNORED, false)`
- `traverse_bfs(..., UsedTag::FROM_TEST)` → `propagate_tags_to_used(..., FROM_TEST, false)`

The report generation loop (`for file in graph.files.iter()`) needs to read tags
from `TagGraph` instead of `GraphFile.file_tags`/`symbol_tags`.

- [ ] Add `source_graph` and `tag_graph` as dependencies of `unused_finder`
- [ ] Replace `Graph::from_source_files` call with `SourceGraph::new`
- [ ] Convert entrypoint paths to `Vec<SegmentKey>` for root seeds
- [ ] Replace 3× `traverse_bfs` calls with `propagate_tags_to_used`
- [ ] Update report generation to read from `TagGraph` + `SourceGraph`
- [ ] Verify existing integration tests pass unchanged

## Remove old `Graph` code

After the new code paths are validated, delete the old types.

- [ ] Remove `Graph`, `GraphFile`, `Edge` from `graph.rs`
- [ ] Remove `symbol_to_segment` field (currently dead code) along with its parent struct
- [ ] Remove `tag_symbol` method from `GraphFile`
- [ ] Clean up any orphaned imports
- [ ] Run `cargo test -p unused-finder` — all tests pass
- [ ] Run `cargo test --workspace` — no regressions
