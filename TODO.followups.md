# Follow-up Issues (unrelated to segment-aware-migration)

- **`import_require_expr.rs` debug `println!`**: Line ~91 has `println!("arg prop: {:?}", prop);` which should be removed or gated behind a debug flag. Left as-is since it's pre-existing.
- **Dead code warnings in `ast_segmenter`**: ~27 warnings for unused structs/fields/functions (e.g. `Visitor`, `ImportKind`, `LazyModule`, etc.). These are WIP scaffolding and expected to be wired in during later phases.
- **Old `Segment`/`SegmentKind` types in `lib.rs`**: Now fully dead code after `visitor.rs` was rewritten to return `RawSegment`. Consider removing or gating behind a feature flag once downstream phases settle.
