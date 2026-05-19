# Notes: swc-to-oxc migration — phase 04 (ast_segmenter)

## `BindingPattern` is the enum, not a wrapper

SWC uses `BindingPat { kind: BindingPatKind::Ident(_) }` (struct + enum). OXC's
`BindingPattern` IS the enum directly:
```rust
match pat {
    BindingPattern::BindingIdentifier(id) => ...  // correct
    // NOT: match pat.kind { ... }
}
```

## `parse_ts` requires `Allocator` first

```rust
let allocator = oxc_allocator::Allocator::default();
let ret = oxc_utils_parse::parse_ts(&allocator, source);
```
For production use with file paths, prefer `parse_file(&allocator, source, path)` which
infers TS vs TSX from the extension.

## `GetSpan` must be imported explicitly

`use oxc_span::GetSpan;` is required to call `.span()` on any OXC AST node.
OXC also exposes `.span` as a direct field on most statement/expression nodes —
use the field directly when possible to avoid the trait import.

## `SymbolTags::from_comments` signature change

Old: `from_comments(comments: impl Comments, lo: BytePos)`
New: `from_comments(comments: &[Comment], source: &str, lo: u32)`

The `source` slice is needed to extract comment text via `Comment::content_span()`:
```rust
let cs = comment.content_span();
let text = &source[cs.start as usize..cs.end as usize];
```

## `ModuleExportName` now has three variants

SWC had two (`Ident`, `Str`). OXC adds `IdentifierReference`. All three must be matched
in `Symbol::from_module_export_name` and `ExportedSymbol::from_module_export_name`.

## `exported_as` normalization for same-name re-exports

When `spec.local.name() == spec.exported.name()`, set `exported_as = None` to preserve
SWC parity (SWC only set `exported_as` when there was a rename). This keeps
`ExportedSymbol` comparisons stable across the migration.

## `ExportsVisitor` does NOT walk children

`import_export_statement.rs` only handles top-level module declarations (import/export).
It does NOT recursively walk expression bodies, so dynamic `import()` calls inside export
declarations are not tracked by `ExportsVisitor`. They are tracked by `ImportRequireVisitor`
in `find_imports_and_requires`, which is called separately for non-module statements.

## `TSImportEqualsDeclaration` routing

`import foo = require('./foo')` is a `Statement::TSImportEqualsDeclaration`, which is a
module-declaration variant in OXC. It goes through `ExportsVisitor` and gets added to
`dynamic_imports` as a `Namespace` import.

## `scope_from_semantic` shim in `ast_name_tracker`

Added `scope_from_semantic(semantic, stmt_span)` as a temporary shim. It iterates all
semantic symbols and filters those whose span is contained within the statement span.
**Known limitation**: includes symbols from nested scopes (e.g., function bodies), not
just the statement's direct scope. This matches the pre-existing behavior well enough for
phase 4; a precise scope-boundary walk can replace it in phase 5.

Mapping of `SymbolFlags` to `HoistingKind`:
- `Import | TypeImport` → `ImportHoisting`
- `Function` → `FunctionHoisting`
- all others → `LetConstHoisting`

## `ImportDeclaration.specifiers` is `Option<Vec<...>>`

- `None` = `import 'foo'` (side-effect import, no specifiers)
- `Some(vec![])` = `import {} from 'foo'` (treated as side-effect, same path)

Both cases add the path to `executed_paths` with no named imports.

## `is_global_require` via semantic

No need for a `require_identifiers: AHashSet<Id>` — semantic already resolved every
`IdentifierReference`. An unresolved reference (no `symbol_id`) that has `name == "require"`
is the global `require`:
```rust
fn is_global_require(semantic: &Semantic<'_>, ident: &IdentifierReference<'_>) -> bool {
    if ident.name != "require" { return false; }
    let Some(ref_id) = ident.reference_id.get() else { return false; };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}
```

## `import().then()` pattern detection

`visit_call_expression` first walks children (which triggers `visit_import_expression`
adding a `Namespace` entry for `import('foo')`), then detects:
```
StaticMemberExpression { property.name == "then", object: ImportExpression { .. } }
```
When found, the `Namespace` entry is replaced with named imports from the `.then()` callback.

## `segment_graph.rs` had `swc_common::Span::default()` references

Two occurrences at lines 408 and 486 needed to be changed to `oxc_span::Span::default()`.
This file retains other swc dependencies (e.g., `swc_atoms::Atom` for `Name`) — those
will be cleaned up in a later phase.

## Downstream breakage in `unused_finder`

Changing `ast_segmenter::segment_file` to take `(logger, program: &Program<'a>, semantic: &Semantic<'a>)`
required updating three files in `unused_finder`:

1. **`parse/exports_visitor_runner.rs`**: Replaced swc parser + `WrapFileLogger::from_swc_source_file`
   with `oxc_utils_parse::parse_file` + `SemanticBuilder` + `WrapFileLogger::new`.
   Source string is cloned for the logger (minor cost: one extra allocation per file).

2. **`parse/data.rs`**: Changed `use swc_common::Span` to `use oxc_span::Span`.
   Also removed the dead `impl From<&swc_ecma_ast::ModuleExportName> for ExportedSymbol`
   — it was defined but never called.

3. **`src/report.rs`**: Removed `use swc_common::source_map::SmallPos` (was used for
   `.lo().to_u32()` / `.hi().to_u32()`, now replaced with `.start` / `.end`).

The tests in `exports_visitor_tests.rs` still use swc for parsing (they test data conversion
logic, not segment_file); that is acceptable for now and will be migrated in phase 5.

## Walk API

OXC provides free functions in `oxc_ast_visit::walk`:
- `walk::walk_statement(visitor, stmt)` — walk a single statement
- `walk::walk_call_expression(visitor, call)` — walk a call expression's children
Override `visit_*` methods on your visitor; the default impls call the walk functions.

## `oxc_utils_parse` version

The local crate is version `0.1.0`, not `0.2.0`. Use `{ version = "0.1.0", path = "../oxc_utils_parse" }`.
