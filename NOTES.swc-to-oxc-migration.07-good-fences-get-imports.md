# Phase 07 – good_fences::get_imports migration notes

## Key design decisions

### `ImportPathVisitor` lifetime parameters
The struct now carries `<'s, 'a>` lifetimes where `'a` is the arena lifetime and
`'s: 'a` or `'s` independent — `semantic: &'s Semantic<'a>`. The `Semantic` type
does not implement `Debug`, so the `#[derive(Debug)]` was removed from the struct.

### `import()` dynamic expressions
In SWC, `import('foo')` was a `CallExpr` with `Callee::Import`. In OXC, it is an
`ImportExpression` node visited via `visit_import_expression`. We **do** walk
children in `visit_import_expression` (via `walk::walk_import_expression`) to
support nested patterns like `import(import('inner').default + '/suffix')`.

### `require` scope detection
The old SWC approach used `visit_binding_ident` to collect binding sites, then
checked `SyntaxContext` equality at call sites. The new approach uses
`is_global_require` from `Semantic::scoping().get_reference(...).symbol_id()`.
Since `var` hoisting makes the entire scope owned by the local binding from a
static analysis perspective, `test_require_redefinition` now expects an **empty**
set (both call sites resolve to the local var-hoisted `require`). This is the
semantically correct behaviour.

### `visit_export_named_declaration` must walk children
Unlike the SWC version which only looked at the `src` field, the OXC version
calls `walk::walk_export_named_declaration` before processing, so that
`require()`/`import()` inside exported declarations are also captured.

### `ImportDeclaration.specifiers` is `Option<Vec<…>>` in OXC
A bare `import './foo'` has `specifiers = None`. We use
`.as_deref().map_or(&[][..], |v| v.as_slice())` to produce an empty slice,
preserving the existing behaviour (bare imports result in an empty specifier
set and are dropped by `get_imports_map_from_visitor`).

### Parse-before-semantic guard
OXC's `SemanticBuilder` must only run on successfully-parsed programs.
We check `ret.panicked || !ret.errors.is_empty()` and reset the arena early
on the error path — same pattern as `unused_finder`.

### `test_parser_error` message
OXC produces different error messages than SWC for the same syntax error.
The test was relaxed to only assert `starts_with("Error parsing <filepath>")`.

### `oxc_utils_parse` is a local path dep
It is not listed under `[workspace.dependencies]`; add as
`{ version = "0.1.0", path = "../oxc_utils_parse" }`.
`oxc_span` was not required (no span types needed in this crate).

## Cargo.toml changes
Removed: `swc_ecma_parser`, `swc_common`, `swc_ecma_ast`, `swc_ecma_visit`,
         `swc_ecma_transforms` (main deps); `swc_utils_parse` (dev dep).
Added:   `oxc_allocator`, `oxc_ast`, `oxc_ast_visit`, `oxc_semantic`,
         `oxc_utils_parse`.
