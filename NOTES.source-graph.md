# Source Graph — Working Notes

## Phase 1: Create `source_graph` crate with core types

### Design decisions

- **`SourceFileInput` instead of `ResolvedSourceFile`**: The TODO spec calls for `SourceGraph::new()` to accept `ResolvedSourceFile`, but that type lives in `unused_finder`. Since `unused_finder` will later depend on `source_graph` (phase 7), accepting it directly would create a circular crate dependency. Introduced `SourceFileInput` as a lightweight input struct with just `source_file_path` and `segments`. When wiring (phase 7), `unused_finder` can construct `SourceFileInput` from `ResolvedSourceFile`.

- **`import_export_info` deferred**: The `SourceGraphFile` struct in the TODO spec includes `import_export_info: ResolvedImportExportInfo`. This field is needed for cross-file resolution (phase 3) but `ResolvedImportExportInfo` also lives in `unused_finder`. Will add it when implementing `resolve_import_across_files` — may need to extract shared types into a separate crate or store a type-erased version.

- **`ExportedSymbol` from `ast_segmenter`**: `symbol_to_segment` uses `ast_segmenter::ExportedSymbol` (Named/Default), not `unused_finder::ExportedSymbol` (which adds Namespace/ExecutionOnly). This is correct since segment-level exports only produce Named/Default variants.

- **`swc_atoms` version 0.6.7**: Matches `ast_name_tracker` which defines `VariableScope`. The `ast_segmenter` crate uses `swc_atoms 5.0.0` for its own types but re-exports `VariableScope` from `ast_name_tracker`, so 0.6.7 is the correct version for the `Atom` type in `name_to_declaring_segments`.

- **Added `VariableScope` accessors**: Added `get_locals_with_hoisting()` and `get_escaped_symbols()` to `ast_name_tracker::VariableScope` — these were needed to build per-file indexes and will be needed for intra-file resolution (phase 2).

### Workspace integration
- Workspace uses `members = ["crates/*"]` glob, so no Cargo.toml edit needed.
- `name_to_declaring_segments` field triggers a dead_code warning since it's not read yet. Will be used by `resolve_symbol_in_file` (phase 2).

## Phase 2: Implement `resolve_symbol_in_file`

### Design decisions

- **Shadowing: LetConst wins over hoisted**: When both a hoisted (import/function) declaration and a visible LetConst declaration exist for the same name, the LetConst declaration takes precedence. This matches JS runtime semantics where `let`/`const` in the same scope shadows hoisted bindings.

- **LetConst: latest-before-reference wins**: When multiple LetConst declarations exist (e.g. from different segments), we pick the latest one at or before `referencing_segment_idx`. This handles sequential `let` rebinding.

- **Hoisted: first declaration wins**: For Import/Function hoisting, the first (lowest segment index) declaration is preferred, since hoisted declarations are visible everywhere and earlier declarations conceptually "win" in JS.

- **`VariableScope::insert_local` added**: Added a public `insert_local(name, hoisting)` method to `VariableScope` for constructing test fixtures without parsing source code. Uses `Span::default()` for the `VarID` since the span isn't relevant for graph resolution.

## Phase 3: Implement `resolve_import_across_files`

### Design decisions

- **`resolved_reexport_paths` on `SourceFileInput`**: Rather than pulling in `ResolvedImportExportInfo` (which lives in `unused_finder` and would create a circular dep), `SourceFileInput` now carries `resolved_reexport_paths: AHashMap<String, PathBuf>` — a map from raw import specifiers (keys of `exports_from` in `RawModuleDeps`) to resolved file paths. Re-export details (what's imported/exported) are already in each segment's `module_deps.exports_from`; we only need resolved paths to follow them.

- **Re-export index built during construction**: `SourceGraphFile` now has `named_reexports` and `star_reexport_paths` built from iterating all segments' `exports_from`. This avoids scanning all segments on every resolution query. `named_reexports` maps exported symbol → Vec of (target path, imported symbol in target). Star paths are deduplicated.

- **Resolution priority**: Direct export (in `symbol_to_segment`) > named re-exports > star re-exports. This matches JS module semantics where explicit named exports shadow star re-exports.

- **Cycle detection via visited set**: The recursive resolver carries `AHashSet<(u32, ExportedSymbol)>` through the call stack. A repeated (file_id, symbol) pair terminates the branch with an empty result. This is passed only through the inner recursion, not cached.

- **Cache at public API boundary**: The `RefCell`-based cache stores final results keyed by `(file_id, ExportedSymbol)`. It is populated only in the public `resolve_import_across_files` entry point, not in inner recursion (intermediate results during chain-following don't need caching since the final result at each entry will be cached on first access). Cache is cleared via `clear_reexport_cache()` — `patch_file` (phase 4) will call this.

- **`export * as Foo from` deferred**: The `(Namespace, Some(name))` case (namespace re-exported under a name) is silently ignored. This is uncommon and would require resolving to "all exports of target file" — complex and not needed for initial correctness.

- **Bonus test**: Added `test_cross_file_renamed_reexport` for the `export { foo as bar }` pattern, verifying that renaming remaps the lookup symbol correctly and the original name doesn't leak through.

## Phase 4: Implement iterators and `patch_file`

### Design decisions

- **`patch_file` takes `SourceFileInput`**: The TODO spec uses `&ResolvedSourceFile`, but that type lives in `unused_finder`, which would create a circular crate dependency. Consistent with the `SourceFileInput` pattern established in phase 1, `patch_file` accepts an owned `SourceFileInput`.

- **`patch_file` supports adding new files**: If the path doesn't exist in the graph, `patch_file` appends the file and updates `path_to_id`. This supports incremental graph construction without requiring all files upfront.

- **Full cache invalidation on patch**: `patch_file` clears the entire re-export cache rather than selectively invalidating entries that touched the patched file. Selective invalidation would require tracking reverse dependencies through the cache, and full invalidation is simple and correct — the cache repopulates lazily on next access.

- **Iterator return types**: `iter_file_segments` returns `Option<impl Iterator>` (None for unknown paths), while `iter_segments` returns `impl Iterator` directly (always valid, possibly empty).

## Phase 5: Create `tag_graph` crate

### Design decisions

- **`UsedTag` duplicated in `tag_graph`**: `UsedTag` (bitflags) is defined in `unused_finder::tag`, but `tag_graph` can't depend on `unused_finder` (circular dep in phase 7 when `unused_finder` depends on `tag_graph`). Duplicated the bitflags definition in `tag_graph`. Phase 7 should migrate `unused_finder` to import `UsedTag` from `tag_graph` instead.

- **`set_tag` unions flags**: `set_tag(key, tag)` ORs the new tag into any existing tag on that segment. This supports multiple propagation passes (entry, test, ignored) tagging the same segment additively, matching `Graph::tag_symbol` behavior in the old code.

- **`file_tag` derives from `SourceGraph` structure**: Uses `SourceGraph::file_segments(file_id)` to discover how many segments a file has, then unions tags across all of them. Returns `UsedTag::empty()` for unknown file IDs (no panic).

- **Workspace auto-discovery**: Workspace uses `members = ["crates/*"]` glob, so no `Cargo.toml` workspace edit needed — same as `source_graph`.

- **Test helpers**: Tests construct `Segment` via `ast_segmenter::raw_module_deps::RawModuleDeps` (public) rather than `segment_info::RawModuleDeps` (re-exported privately). These are dev-dependencies only.

## Phase 6: Implement `propagate_tags_to_used` (downward)

### Design decisions

- **`resolved_import_paths` on `SourceFileInput`**: The TODO spec references `ResolvedImportExportInfo::iter_imported_symbols_meta` for discovering inter-file imports, but that type lives in `unused_finder`. Instead, `SourceFileInput` now carries `resolved_import_paths: AHashMap<String, PathBuf>` alongside the existing `resolved_reexport_paths`. This maps raw import specifiers (keys of `imports`, `dynamic_imports`, `requires`, `executed_paths` in `RawModuleDeps`) to resolved file paths. Phase 7 wiring will populate this from the resolver.

- **`ast-segmenter` promoted to runtime dep**: `tag_graph` now depends on `ast-segmenter` (not just dev-dep) because `propagate_tags_to_used` references `ast_segmenter::raw_module_deps::Symbol` to convert import symbols to `ExportedSymbol` for resolution.

- **Symbol → ExportedSymbol mapping**: `Symbol::Named` → `ExportedSymbol::Named`, `Symbol::Default` → `ExportedSymbol::Default`, `Symbol::Namespace` → tag all segments in target file (namespace import touches everything).

- **Side-effect imports and requires**: `executed_paths` (`import './foo'`) and `requires` (`require('./foo')`) tag all segments in the target file, since they don't import specific symbols but execute the module for its side effects / expose everything.

- **`insert_escaped` on `VariableScope`**: Added public `insert_escaped(name)` method to `ast_name_tracker::VariableScope` for constructing test fixtures with escaped symbol references, matching the existing `insert_local` pattern.

- **BFS visited set**: Uses `AHashSet<SegmentKey>` for cycle detection. Segments are only enqueued once, preventing infinite loops in circular import graphs. The visited set also prevents re-tagging already-tagged segments.

- **`is_type_only` filtering**: Only static imports (`module_deps.imports`) carry `SymbolTags` with `is_type_only`. Dynamic imports, requires, and executed paths don't have type-only semantics and are always followed.

## Phase 7: Implement `propagate_tags_to_users` (upward)

### Design decisions

- **Reverse edges built on demand**: Rather than caching the reverse edge index in `SourceGraph` (which would require invalidation on `patch_file`), `build_reverse_edges` is a private helper called at the start of each `propagate_tags_to_users` invocation. This is simple and correct — the index is O(edges) to build and only needed when upward propagation is requested. If performance becomes a concern, it can be cached with invalidation later.

- **Mirrors forward traversal logic**: `build_reverse_edges` walks the same edge types as `propagate_tags_to_used` (static imports, dynamic imports, requires, executed paths, intra-file escaped symbols) but records the reverse direction. This ensures the two propagation modes are symmetric over the same edge set.

- **`follow_type_only` applied during index construction**: Type-only edges are filtered when building the reverse index (not during BFS traversal). This keeps the BFS loop simple — it only needs to look up reverse edges and enqueue unvisited targets.
