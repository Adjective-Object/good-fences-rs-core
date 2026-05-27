# Notes: swc-to-oxc-migration phase 04 — ast_segmenter

## Section 1: Port data types

### Span migration
- `Segment.span`, `TaggedSymbol.span`, `ReExportedSymbol.span` now use `oxc_span::Span`
- `oxc_span::Span` has `start: u32` / `end: u32` fields (no `lo()`/`hi()` methods)
- Downstream callers in `unused_finder/src/report.rs` that used `.span.lo().to_u32()` updated to `.span.start`

### from_comments signature
- `from_comments(comments: &[Comment], source: &str, lo: u32)` — needs source text to extract comment text from spans
- `leading_comments_at` from `oxc_utils_parse` replaces `Comments::get_leading(BytePos)`
- Oxc `Comment.span` includes the `//` delimiter; skip first 2 bytes for line comments before checking content

### Bridge pattern (temporary)
- `import_export_statement.rs` still uses the swc visitor (`Visit` trait). Three temporary bridge helpers:
  - `exported_symbol_from_swc_export_name` — converts swc `ModuleExportName` → `ExportedSymbol`
  - `symbol_from_swc_export_name` — converts swc `ModuleExportName` → `Symbol`
  - `to_oxc_span` — wraps `logger_srcfile::swc_compat::swc_span_to_oxc`
- `ExportsVisitor` now carries both `swc_comments: &SingleThreadedComments` (unused, kept for type compat) and `comments: &[Comment]` + `source: &str`
- `segment_file()` signature gains `comments: &[Comment]` and `source: &str` parameters
- Test helpers double-parse with both swc and oxc to get oxc comments alongside swc AST
- These bridges will be removed when the visitor is fully ported to oxc (section 2)

### From<&swc_atoms::Atom> removal
- Removed; `From<&str>` already existed and oxc's `Ident` provides `.as_str()` → `&str`

### from_module_export_name
- Oxc has 3 variants: `IdentifierName`, `IdentifierReference`, `StringLiteral`
- All three extract name via `.name.as_str()` (ident variants) or `.value.as_ref()` (string)

### Downstream crate updates
- `source_graph`, `tag_graph`: test helpers use `oxc_span::Span::new(lo, hi)` instead of `swc_common::Span::new(BytePos(lo), BytePos(hi))`
- `unused_finder`: `ExportedSymbolMetadata.span` changed from `swc_common::Span` to `oxc_span::Span`; `report.rs` updated for field access
