# Notes: Phase 7 — good_fences get_imports

## Key decisions

- **Thread-local arena**: same `PARSE_ARENA` pattern as `unused_finder::get_file_segments`.
  Arena is reset after each file parse; all visitor data is owned (`String`/`HashSet`).

- **`import()` is `ImportExpression`, not `CallExpression`**: in oxc, dynamic
  `import()` is a separate AST node. Added `visit_import_expression` instead of
  checking `expr.callee.is_import()` inside `visit_call_expression`.

- **`var require` hoisting behavior change**: `test_require_redefinition` expected
  `before_definition` to be treated as a global `require` call. But `var require`
  hoists to function scope, so oxc_semantic correctly resolves *all* `require`
  references in that scope to the local variable. Updated the test to expect an
  empty set. This is semantically more correct than the old swc + resolver behavior.

- **Parser error message difference**: oxc reports "Expected a semicolon or an
  implicit semicolon after a statement, but found none" vs swc's "Expected ';',
  '}' or <eof>". Updated the `test_parser_error` assertion.

- **`Semantic` is not `Debug`**: implemented `Debug` manually for
  `ImportPathVisitor` using `finish_non_exhaustive()`.

- **Test macro**: introduced `with_parsed_visitor!` macro to manage the allocator
  + semantic lifetimes within each test, since the visitor borrows from both.

- **`ModuleExportName::IdentifierReference`**: oxc has three variants vs swc's
  two. Added `module_export_name_to_str` helper covering all three, matching the
  pattern in `ast_segmenter`.
