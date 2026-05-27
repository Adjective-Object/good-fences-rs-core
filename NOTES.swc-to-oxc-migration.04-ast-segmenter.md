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

## Section 2: Port visitors and top-level segmenter

### import_export_statement.rs
- Full rewrite from `swc_ecma_visit::Visit` to `oxc_ast_visit::Visit<'a>`
- `ExportsVisitor` now takes `&Semantic<'a>`, `&[Comment]`, `&str` instead of `&SingleThreadedComments`
- Dropped `require_identifiers: AHashSet<Id>` + `visit_binding_ident`; replaced with `is_global_require()` using semantic scope resolution
- `import()` in oxc is `ImportExpression`, NOT a `CallExpression` — needs separate `visit_import_expression` handler (not just checking `callee`)
- `get_export_bindings`: when local == exported name, set `exported_as: None` to match swc behavior (swc omitted rename when names matched)
- Removed temporary bridge helpers (`to_oxc_span`, swc export name converters)

### import_require_expr.rs
- `ImportsAndRequiresVisitor` wraps `ImportsAndRequires` + `&Semantic` for the Visit impl
- Added `visit_import_expression` for standalone `import('foo')` → Namespace
- `visit_call_expression` handles `require('foo')` and `import('foo').then(cb)` pattern
- For `.then()` pattern: oxc AST has `StaticMemberExpression.object = ImportExpression` directly (not wrapped in `CallExpression`)
- Walk order ensures `visit_import_expression` fires first (adding Namespace), then `scan_call_expr` replaces with Named entries for `.then()` destructuring

### visitor.rs (top-level segmenter)
- `segment_file` now takes `&Program<'a>` + `&Semantic<'a>`
- `find_names_from_semantic()` shim extracts root-scope bindings into `VariableScope` — intentionally returns ALL program-level bindings for every statement (lossy but acceptable as bridge; phase 4 removes ast_name_tracker)
- Simplified match: no exhaustive variant listing, just module-decl arms + error arms + wildcard
- `Statement::UsingDeclaration` doesn't exist in oxc — `using` is `VariableDeclaration { kind: Using }`

### oxc_utils_parse
- `parse_file` now always uses TypeScript source type (matching swc's `Syntax::Typescript`), since many codebases use TS syntax in `.js` files
- `.tsx`/`.jsx` extensions get `SourceType::tsx()`, everything else gets `SourceType::ts()`

### unused_finder (downstream)
- `exports_visitor_runner.rs`: rewritten to use oxc parsing + SemanticBuilder
- `exports_visitor_tests.rs`: parse() helper uses oxc
- `source.clone()` needed for WrapFileLogger since source is borrowed by parser

### Cargo.toml updates
- ast_segmenter: added `oxc_allocator`, `oxc_ast_visit`, `oxc_semantic`; swc_atoms pinned to 0.6.7 (must match ast_name_tracker)
- unused_finder: added `oxc_allocator`, `oxc_semantic`, `oxc_utils_parse`
