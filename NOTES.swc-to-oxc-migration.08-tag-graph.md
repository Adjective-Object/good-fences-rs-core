# Phase 08 – tag_graph: SWC → OXC migration notes

## Status

All items already completed by prior phases. This phase was a no-op.

## Details

- `tag_graph` production code never depended on swc — it uses `source_graph`
  and `ast_segmenter` types only.
- Test code was migrated in an earlier phase (likely phase 4 or 7) when
  `ast_segmenter::Segment` switched its `span` field to `oxc_span::Span`.
- `Cargo.toml` already has `oxc_span` as a dev-dependency and no swc
  dependencies.
- Tests use `compact_str::CompactString` (not `swc_atoms::Atom`) for escaped
  symbol construction.
- All 16 tests pass. `./scripts/spot-check` passes clean.
