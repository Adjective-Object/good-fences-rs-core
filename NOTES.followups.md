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
