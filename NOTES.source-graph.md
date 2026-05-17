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
