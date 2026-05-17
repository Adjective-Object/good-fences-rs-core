# Follow-up Issues (unrelated to segment-aware-migration)

## delete dead code

- [x] **`import_require_expr.rs` debug `println!`**: Line ~91 has `println!("arg prop: {:?}", prop);` which should be removed

## Delete dead segment/segmentkind types

- [x] **Old `Segment`/`SegmentKind` types in `lib.rs`**: Now fully dead code after `visitor.rs` was rewritten to return `RawSegment`. Consider removing or gating behind a feature flag once downstream phases settle.
- [x] Rename old RawSegment type to Segment

## Finish migration

- [x] **`unused_finder::ExportsVisitor` now unused from main pipeline**: The `ExportsVisitor` in `unused_finder/src/parse/exports_visitor.rs` is no longer called from the production pipeline (replaced by `ast_segmenter::segment_file`). It's still referenced by `exports_visitor_tests.rs`.
- [x] Migrate tests to use `ast_segmenter` directly, or keeping it as a reference implementation.

## Dead code cleanup (discovered during migration)

- [x] **`RawModuleDeps` re-export unused**: `unused_finder/src/parse/data.rs:14` re-exports `ast_segmenter::raw_module_deps::RawModuleDeps` but nothing in the crate uses it.
- [x] **`get_file_import_export_info` is dead code**: `unused_finder/src/parse/exports_visitor_runner.rs` — all callers now use `get_file_segments` directly.
- [x] **`Dependencies2D` fields never read**: `ast_segmenter/src/visitor.rs` — the struct and its methods are defined but never used.

## Pre-existing warnings (discovered during dead code cleanup)

- [ ] **`RawModuleDeps` accessor methods never used**: `ast_segmenter/src/raw_module_deps.rs:244-261` — six private getter methods (`imports`, `dynamic_imports`, `requires`, `exports_from`, `exports_locals`, `executed_paths`) are dead code since all callers access the `pub` fields directly.
- [ ] **`get_file_segments` re-export unused from `parse/mod.rs`**: Callers import directly from `exports_visitor_runner` submodule instead of through the re-export.
- [ ] **`default_spec` unused variable**: `ast_segmenter/src/visitors/import_export_statement.rs:236` — should be prefixed with `_`.
- [ ] **Non-shorthand field pattern**: `ast_segmenter/src/visitors/import_require_expr.rs:64` — `args: ref args` should be `ref args`.
