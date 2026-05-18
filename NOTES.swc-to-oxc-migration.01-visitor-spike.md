# Notes: swc-to-oxc-migration.01-visitor-spike

Working notes from the visitor spike (phase 1). Each item corresponds to a
check in `TODO.swc-to-oxc-migration.01-visitor-spike.md`.

---

## oxc_ast_visit shape

- **CONFIRMED** `oxc_ast_visit::Visit<'a>` exists and has per-node `visit_*`
  hooks for all imports/exports needed in phase 3 (e.g. `visit_import_declaration`,
  `visit_export_named_declaration`, `visit_export_all_declaration`,
  `visit_import_expression`, `visit_ts_import_equals_declaration`).

- **CONFIRMED** `walk_statement` exists as a free function:
  `pub fn walk_statement<'a, V: Visit<'a>>(visitor: &mut V, it: &Statement<'a>)`
  exported from `oxc_ast_visit::walk`. This lets phase 3's
  `find_imports_and_requires(stmt)` walk a single `Statement` directly.

- **CONFIRMED** visiting a `&Statement` from a `Program` body requires no
  builder/state setup beyond constructing the visitor struct.

---

## AST enum shapes

- **CONFIRMED** `ModuleExportName<'a>` has exactly three variants:
  `IdentifierName(IdentifierName<'a>)` (= 0),
  `IdentifierReference(IdentifierReference<'a>)` (= 1),
  `StringLiteral(StringLiteral<'a>)` (= 2).
  Phase 3's assumption matches.

- **CONFIRMED** Dynamic `import(x)` is `Expression::ImportExpression(_)`
  wrapping `ImportExpression<'a> { source: Expression, .. }`.
  Phase 3 was correct; the `source` field is itself an `Expression`.

- **CORRECTED** `export * from 'mod'` and `export * as ns from 'mod'` both
  map to `ExportAllDeclaration` (not `ExportNamedDeclaration`). The field
  `exported: Option<ModuleExportName<'a>>` holds the namespace alias for the
  `export * as ns` form, or is `None` for a bare `export *`. There is **no**
  `ExportNamespaceSpecifier` variant anywhere in OXC. Phase 3's mapping table
  must be updated to use `ExportAllDeclaration { exported: Some(...) }`.

- **CORRECTED** `export v from 'mod'` (swc's `ExportSpecifier::Default`) is
  **not** parsed by OXC at the pinned rev. OXC's `ExportSpecifier` only has
  `local: ModuleExportName<'a>` and `exported: ModuleExportName<'a>` — no
  Default variant. Good-fences will silently stop recognising that stage-1
  syntax; no fixtures use it (confirmed by grep).

- **CORRECTED** Identifier field `name` is `Ident<'a>` (from `oxc_str`), **not**
  `Atom<'a>`. `Ident<'a>` implements `Deref<Target = str>`, so `&*ident.name`
  or `.name.as_str()` gives a `&str`. There is no `Atom` type in this rev of
  OXC.
  `reference_id` on `IdentifierReference` is a `Cell<Option<ReferenceId>>`
  **field**, but a generated panicking accessor `fn reference_id(&self) ->
  ReferenceId` is available (via `oxc_ast::generated::get_id`). The accessor
  panics if semantic has not run. Safe access: `r.reference_id.get()` returns
  `Option<ReferenceId>`.

- **CONFIRMED** `TSImportEqualsDeclaration` fields:
  `id: BindingIdentifier<'a>`, `module_reference: TSModuleReference<'a>`,
  `import_kind: ImportOrExportKind`.

---

## Semantic API

- **CONFIRMED** `Semantic::scoping()` exists and returns `&Scoping`.
  `Scoping` provides:
  - `get_reference(ReferenceId) -> &Reference`
  - `get_bindings(ScopeId) -> &Bindings<'_>` where `Bindings<'a>` is
    `ArenaIdentHashMap<'a, SymbolId>`
  - `symbol_ids() -> impl Iterator<Item = SymbolId>`
  
  **CORRECTED** There is **no** plain `references()` iterator returning all
  references by value. To iterate all references use `scoping.symbol_ids()`
  and then `scoping.get_resolved_references(sym_id)` per symbol, or walk the
  AST nodes via `Semantic::nodes().iter()` and match on
  `AstKind::IdentifierReference`.

- **CONFIRMED** `IdentifierReference::reference_id()` — the generated
  accessor panics if `None` (semantic not run). Safe form:
  `r.reference_id.get() -> Option<ReferenceId>`. Phase 3/4 should use
  `r.reference_id.get()` with a `.map(|id| scoping.get_reference(id))`.

- **CONFIRMED** Per-symbol declaration span: `Scoping::symbol_span(SymbolId)
  -> Span`.

- **CONFIRMED** `SymbolFlags` variants: `Import`, `Function`,
  `BlockScopedVariable`, `FunctionScopedVariable`, `Class`, `TypeAlias`,
  `Interface`. **CORRECTED** `Enum` is a composite alias (`ConstEnum |
  RegularEnum`), not a single-bit flag. Phase 4 should use
  `flags.intersects(SymbolFlags::Enum)` rather than equality or
  `flags.contains(SymbolFlags::Enum)`.

---

## Span ergonomics

- **CORRECTED** `Span` does **not** have `contains_span`. The correct method
  is `contains_inclusive(other: Span) -> bool`. Phase 4's bucketing code must
  call `.contains_inclusive(...)` instead of `.contains_span(...)`.
  `start: u32` and `end: u32` are pub fields — confirmed.

- **CONFIRMED** `SourceType::from_path("comp.tsx")` sets `is_jsx() == true`
  (`.tsx` maps to `LanguageVariant::Jsx`). Confirmed at runtime in spike.

---

## Corrections applied

The following items in `PLAN.swc-to-oxc-migration.md` and phase TODOs were
updated based on the above corrections:

1. `export * as ns` shape: `ExportAllDeclaration.exported: Option<ModuleExportName>`
   (no `ExportNamespaceSpecifier`).
2. `export v from 'mod'` (stage-1): not supported by OXC; document as silent
   drop.
3. `name` field type: `Ident<'a>` (derefs to `&str`), not `Atom<'a>`.
4. `reference_id`: cell field + generated accessor; use `r.reference_id.get()`
   for safe access.
5. No plain `references()` iterator; walk via `symbol_ids()` +
   `get_resolved_references()`.
6. `Span::contains_inclusive` not `contains_span`.
7. `SymbolFlags::Enum` is a composite; use `intersects`.
