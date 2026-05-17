# Follow-up Notes

## delete dead code

### `import_require_expr.rs` debug `println!` removal

Removed `println!("arg prop: {:?}", prop);` at line 91 of
`crates/ast_segmenter/src/visitors/import_require_expr.rs`.

This was a leftover debug print inside a `filter_map` closure that processes
object destructuring patterns from `require()` calls. No other stray `println!`
calls remain in the `ast_segmenter` crate. All spot checks pass.

## Delete dead segment/segmentkind types

### Removed dead types from `ast_segmenter/src/lib.rs`

Deleted the following unused types: `Segment` (old), `SegmentKind`, `ImportKind`,
`NormalSegment`, `ExportLocal`, `ImportedLocal`, `NormalSegmentImportInfo`,
`LazyModule`, `LazyModuleReference`, `StaticImportType`, `ModuleImport`.
Also removed large block of design-notes comments that referenced these types.

Kept live types: `ImportTarget`, `ExportedSymbol`, `ReExportedSymbol` (and impls) —
these are used across `ast_segmenter` and `unused_finder`.

Removed the unused `use ast_name_tracker::VariableScope` import from `lib.rs`
(was only needed by the old `Segment` struct; `VariableScope` is still used in
`segment_info.rs` where the real type lives).

### Renamed `RawSegment` → `Segment`

Renamed in `segment_info.rs` (definition) and all 8 consuming files across
`ast_segmenter` and `unused_finder`. The `pub use segment_info::Segment` re-export
from the crate root is unchanged in shape.

No behavioral changes — purely a naming cleanup. Pre-existing warnings
(`Dependencies2D`, `RawModuleDeps` accessor methods, `ExportsVisitor::new`) are
unchanged and belong to the next TODO section ("Finish migration").

## Finish migration

### Deleted `unused_finder`'s `ExportsVisitor` and migrated tests to `ast_segmenter`

The old `ExportsVisitor` in `unused_finder/src/parse/exports_visitor.rs` was a
full SWC `Visit` impl that walked an entire module to collect import/export info
into `RawImportExportInfo`. The production pipeline had already moved to
`ast_segmenter::segment_file` (via `exports_visitor_runner.rs`), leaving the old
visitor used only by `exports_visitor_tests.rs`.

**What changed:**
- Deleted `exports_visitor.rs` entirely (456 lines).
- Removed the `pub mod exports_visitor` declaration from `parse/mod.rs`.
- Rewrote `exports_visitor_tests.rs` to parse via
  `ast_segmenter::segment_file` → `RawImportExportInfo::from(segments.as_slice())`
  instead of constructing an `ExportsVisitor` directly.
- The `parse()` test helper uses `create_lexer` + `Capturing` + `Parser`
  (matching `exports_visitor_runner.rs`) instead of the old `create_parser`.

All 35 existing test cases pass unchanged — the `ast_segmenter` pipeline
produces identical `RawImportExportInfo` for every test input.

**Pre-existing warnings observed (not addressed):**
- `RawModuleDeps` re-export is unused (`data.rs:14`)
- `get_file_import_export_info` is never used (dead code after callers moved to
  `get_file_segments`)
- `Dependencies2D` fields/methods are never read in `ast_segmenter::visitor.rs`

These are candidates for future cleanup but are unrelated to this migration.

## Dead code cleanup (discovered during migration)

### Removed `RawModuleDeps` re-export from `unused_finder/src/parse/data.rs`

Line 14 had `pub use ast_segmenter::raw_module_deps::RawModuleDeps;`. No code in
`unused_finder` ever referenced this re-export — callers that need `RawModuleDeps`
import it directly from `ast_segmenter`. Removed the single line.

### Removed `get_file_import_export_info` from `unused_finder/src/parse/exports_visitor_runner.rs`

This function was a thin wrapper: `get_file_segments()` → `RawImportExportInfo::from()`.
After the segment-aware migration, all production callers switched to `get_file_segments`
directly. No tests or external code referenced it. Also removed the now-unused
`use crate::parse::RawImportExportInfo` import and the re-export from `parse/mod.rs`.

### Removed `Dependencies2D` from `ast_segmenter/src/visitor.rs`

The struct and its `impl` block (lines 137–160) were scaffolding for a future
segment dependency graph feature. It was never instantiated anywhere. Its only
external dependency was the `roaring` crate (for `RoaringBitmap`), which was also
unused elsewhere — removed `roaring = "0.10"` from `Cargo.toml` (both the active
dep and the commented-out dev-dep).

### Pre-existing warnings noted for follow-up

Spotted during compilation but left untouched (unrelated to the dead code task):
- `RawModuleDeps` has six private accessor methods that are never called (fields are `pub`).
- `get_file_segments` re-export in `parse/mod.rs` is unused (callers use the submodule path).
- `default_spec` unused variable and `args: ref args` non-shorthand pattern in `ast_segmenter`.

## Pre-existing warnings (discovered during dead code cleanup)

### Removed `RawModuleDeps` dead accessor methods

Deleted the entire `impl RawModuleDeps` block (6 private getter methods: `imports`,
`dynamic_imports`, `requires`, `exports_from`, `exports_locals`, `executed_paths`).
All fields are `pub`, so callers access them directly. No code ever called these methods.

### Removed unused `get_file_segments` re-export from `parse/mod.rs`

The `pub use exports_visitor_runner::get_file_segments;` line was never used — both
callers (`walk.rs` and `unused_finder.rs`) import via the full submodule path
`crate::parse::exports_visitor_runner::get_file_segments`.

### Fixed `default_spec` unused variable warning

Prefixed with underscore: `ExportSpecifier::Default(default_spec)` →
`ExportSpecifier::Default(_default_spec)` in `import_export_statement.rs:236`.

### Fixed non-shorthand field pattern

Changed `args: ref args` to `ref args` in `import_require_expr.rs:64`.
This is the idiomatic Rust shorthand when binding name matches field name.
