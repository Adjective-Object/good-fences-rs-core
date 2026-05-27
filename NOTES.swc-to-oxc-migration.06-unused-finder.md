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
