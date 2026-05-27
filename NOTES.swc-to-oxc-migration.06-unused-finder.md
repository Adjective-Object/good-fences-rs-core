# Phase 06 — unused_finder: working notes

## Rewrite `get_file_segments` (done)

- The swc parser stack was already removed from `exports_visitor_runner.rs` in
  prior phases; this file was already using `oxc_allocator` / `oxc_utils_parse`
  / `oxc_semantic`. The remaining work was adding the thread-local arena pattern
  and the `panicked` check.
- `PARSE_ARENA` is a `RefCell<Allocator>` in a `thread_local!`. The borrow is
  held during parse+semantic+segmenter, then dropped before `reset()`.
- `Allocator::allocated_bytes()` does not exist at the pinned rev;
  `Allocator::capacity()` is used in the arena-reuse test instead. After
  `reset()`, capacity stays stable for equally-sized inputs.
- `ParserReturn.panicked` maps to `SourceFileParseError::TypeScriptSyntax`;
  parse errors (non-empty `errors` vec) map to `SourceFileParseError::Parser`.
  This differs from the old code which mapped parse errors to
  `TypeScriptSyntax` — the TODO template specifies `Parser` for that path.
- `test_typeonly_interface_allowed` is a pre-existing failure (unrelated to
  this change).

## Drop SmallPos from report.rs (done — was already clean)

- `report.rs` had no `SmallPos` import or `.to_u32()` calls; prior phases
  already migrated spans to `oxc_span::Span` with direct `.start` / `.end`
  field access.

## Update data.rs (done)

- Removed `use swc_ecma_ast::ModuleExportName` and the `From<&ModuleExportName>`
  impl. The impl was dead code — no call sites existed. The ast_segmenter
  already uses `oxc_ast::ast::ModuleExportName` internally; unused_finder
  doesn't need it since it works with owned `ExportedSymbol` mirrors.
- `use oxc_span::Span` and `import_resolver::resolve::PathResolver` were
  already in place from prior phases.

## Update unused_finder.rs (done — was already clean)

- Already using `import_resolver::resolve::{PathResolver, TargetEnv}`.
  No swc imports remained.

## Cargo.toml cleanup (done)

- Removed: `swc_common`, `swc_ecma_ast`, `swc_ecma_loader`, `swc_ecma_parser`,
  `swc_ecma_visit`, `swc_ecma_transforms`, `swc_utils_parse`.
- Dropped `features = ["swc-compat"]` from `logger_srcfile` dep.
- `oxc_allocator`, `oxc_semantic`, `oxc_span`, `oxc_utils_parse` were already
  present. `oxc_ast` not added — unused_finder doesn't reference oxc AST types
  directly (ast_segmenter owns that boundary).
