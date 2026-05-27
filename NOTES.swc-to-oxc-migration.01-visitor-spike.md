# NOTES: swc-to-oxc-migration.01-visitor-spike

Working notes from the phase 0.5 visitor spike. Each item corresponds to an
assumption from `TODO.swc-to-oxc-migration.01-visitor-spike.md`.

## oxc_ast_visit shape

- **confirmed**: `Visit<'a>` trait exists with all required hooks: `visit_import_declaration`,
  `visit_export_named_declaration`, `visit_export_default_declaration`,
  `visit_export_all_declaration`, `visit_call_expression`,
  `visit_ts_import_equals_declaration`, `visit_import_expression`.
- **confirmed**: `walk::walk_statement(&mut V, &Statement)` exists as a free function.
  Walking a single statement works with no builder/state setup beyond constructing the visitor.
- **confirmed**: No builder/state setup needed — just construct the visitor and call
  `walk::walk_statement`.

## AST enum shapes

- **confirmed**: `ModuleExportName` has three variants: `IdentifierName(IdentifierName<'a>)`,
  `IdentifierReference(IdentifierReference<'a>)`, `StringLiteral(StringLiteral<'a>)`.
  Note: phase 3 said `IdentifierReference` which is correct.
- **confirmed**: Dynamic `import(x)` is `Expression::ImportExpression(Box<'a, ImportExpression<'a>>)`.
  `ImportExpression` has fields: `span`, `source: Expression`, `options: Option<Expression>`,
  `phase: Option<ImportPhase>`. Phase 3's guess of `Expression::Import` was wrong — it's
  `Expression::ImportExpression`.
- **corrected**: `export * as ns from 'mod'` is **not** `ExportNamespaceSpecifier`. It's
  `ExportAllDeclaration` with `exported: Option<ModuleExportName>`. When `exported` is `Some`,
  it's `export * as <name>`. When `None`, it's plain `export *`. No separate specifier type.
- **confirmed**: `export v from 'mod'` (stage-1 syntax, swc's `ExportSpecifier::Default`) is
  **not parsed** by OXC at this rev — parser emits "Unexpected token". No fixtures use this
  syntax, so no impact.
- **corrected**: Identifier field type is `name: Ident<'a>`, not `Atom<'a>` or `&'a str`.
  `Ident<'a>` provides `.as_str() -> &str`. `BindingIdentifier` also has
  `symbol_id: Cell<Option<SymbolId>>`. `IdentifierReference` has
  `reference_id: Cell<Option<ReferenceId>>`.
- **confirmed**: `TSImportEqualsDeclaration` fields: `id: BindingIdentifier<'a>`,
  `module_reference: TSModuleReference<'a>`, `import_kind: ImportOrExportKind`.

## Semantic API

- **confirmed**: `Semantic::scoping() -> &Scoping` exists. `Scoping` has:
  - `get_reference(ReferenceId) -> &Reference`
  - `references_len() -> usize` (no `references()` iterator, but length is available)
  - `get_bindings(ScopeId) -> &Bindings` (iterable)
- **corrected**: `IdentifierReference` has a **field** `reference_id: Cell<Option<ReferenceId>>`,
  not a method `reference_id()`. Access via `.reference_id.get()`.
- **confirmed**: `Scoping::symbol_span(SymbolId) -> Span` exists.
- **corrected**: `SymbolFlags` variants differ slightly from assumption:
  - `Import` ✓, `Function` ✓, `BlockScopedVariable` ✓, `FunctionScopedVariable` ✓
  - `Class` ✓, `TypeAlias` ✓, `Interface` ✓
  - `Enum` does NOT exist as a single flag — split into `RegularEnum` and `ConstEnum`.
    Phase 4's hoisting map must use `RegularEnum | ConstEnum` for enum detection.
- **note**: `SymbolFlags` and `SymbolId` are re-exported from `oxc_semantic`, not directly
  from `oxc_span` or `oxc_syntax`. Import as `use oxc_semantic::{SymbolFlags, SymbolId}`.

## Span ergonomics

- **corrected**: Span containment method is `contains_inclusive(Span) -> bool`, **not**
  `contains_span()`. `start: u32` and `end: u32` are public fields. There's a private
  `_align: PointerAlign` field too, but it doesn't affect usage.
- **confirmed**: `SourceType::from_path("x.tsx")` returns a `SourceType` with `is_jsx() = true`.

## Other findings

- **corrected**: `oxc_span::CompactStr` is **private** at this rev. The PLAN says to use
  `CompactStr` as the owned identifier type; this needs updating. Options:
  1. Add `oxc_str` as a workspace dep and use `oxc_str::CompactStr` directly.
  2. Use `String` everywhere (simpler, no extra dep).
  Decision deferred to phase 3 implementor; both work.
- Walk functions live under `oxc_ast_visit::walk::*` (e.g. `walk::walk_statement`), not at
  the crate root.
