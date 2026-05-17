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
