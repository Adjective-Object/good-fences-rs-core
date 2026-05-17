# Follow-up Issues (unrelated to segment-aware-migration)

## delete dead code

- [x] **`import_require_expr.rs` debug `println!`**: Line ~91 has `println!("arg prop: {:?}", prop);` which should be removed

## Delete dead segment/segmentkind types

- [x] **Old `Segment`/`SegmentKind` types in `lib.rs`**: Now fully dead code after `visitor.rs` was rewritten to return `RawSegment`. Consider removing or gating behind a feature flag once downstream phases settle.
- [x] Rename old RawSegment type to Segment

## Finish migration

- [ ] **`unused_finder::ExportsVisitor` now unused from main pipeline**: The `ExportsVisitor` in `unused_finder/src/parse/exports_visitor.rs` is no longer called from the production pipeline (replaced by `ast_segmenter::segment_file`). It's still referenced by `exports_visitor_tests.rs`.
- [ ] Migrate tests to use `ast_segmenter` directly, or keeping it as a reference implementation.
