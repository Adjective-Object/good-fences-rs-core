High level goal: lock down the unknowns about `oxc_ast_visit`, `oxc_ast`
enum shapes, and the semantic reference-id wiring **before** the segmenter
port begins. This is a throwaway spike that lives in a branch — its only
output is a written confirmation/correction of the API assumptions baked
into phases 3, 4, and 6.

Depends on phase 0.

## Spike code

Add a temporary `crates/oxc_spike/` binary (deleted at end of phase). It
parses one `.ts` and one `.tsx` fixture and exercises every API call site
that phases 3/4/6 assume.

```rust
fn main() {
    let allocator = oxc_allocator::Allocator::default();
    let source = std::fs::read_to_string("fixture.ts").unwrap();
    let ret = oxc_utils_parse::parse_file(&allocator, &source, std::path::Path::new("fixture.ts"));
    let semantic = oxc_semantic::SemanticBuilder::new().build(&ret.program).semantic;

    // Exercise every shape we depend on; print results.
    // ...
}
```

## Items to confirm or correct

For each item below, the output is a one-line **confirmed** / **corrected**
note pinned into a new `NOTES.swc-to-oxc-migration.md` (created here).
Where the assumption is wrong, update the corresponding line in
`PLAN.swc-to-oxc-migration.md` and the affected phase TODO.

### `oxc_ast_visit` shape

- [x] Confirm: `oxc_ast_visit::Visit<'a>` exists and has the per-node
      `visit_*` hooks named in phase 3's translation table.
- [x] Confirm: there is a `walk_statement(&mut Visitor, &Statement)` (or
      equivalent free function) so `find_imports_and_requires(stmt)` in
      `ast_segmenter::visitors::import_require_expr` can walk a single
      `Statement` rather than a whole `Program`. If not, change the API to
      take `&Program` and a span filter instead.
- [x] Confirm: visiting a `&Statement` from the body of a `Program` does
      not require any builder/state setup beyond constructing the visitor.

### AST enum shapes

- [x] Confirm the exact name and variants of `ModuleExportName`. Phase 3
      assumes three variants including `IdentifierReference`.
- [x] Confirm dynamic `import(x)` shape. Phase 3 originally said
      `Expression::Import` but that is likely wrong; verify against the
      pinned rev whether it is `Expression::ImportExpression(_)` or a
      `CallExpression` whose callee is some import marker.
- [x] Confirm `ExportNamedDeclaration` shape for the `export *` and
      `export * as ns from 'mod'` cases. Phase 3's mapping table guesses
      `ExportNamespaceSpecifier`; check the actual variant.
- [x] Confirm whether `export v from 'mod'` (swc's `ExportSpecifier::Default`)
      is even parsed by oxc at the pinned rev. If not, document that
      good-fences will silently stop recognising that stage-1 syntax and
      verify no fixtures use it.
- [x] Confirm `BindingIdentifier`, `IdentifierReference`, `IdentifierName`
      field names (`name: Atom<'a>` vs `name: &'a str`).
- [x] Confirm `TSImportEqualsDeclaration` field names (`module_reference`,
      `id`, etc.).

### Semantic API

- [x] Confirm the existence of `Semantic::scoping()` and that the returned
      type has `get_reference(ReferenceId) -> &Reference`,
      `references()` iterator, and `get_bindings(ScopeId)` (or equivalent
      "bindings declared in this scope").
- [x] Confirm the `IdentifierReference` node carries a `reference_id()`
      accessor that returns a `ReferenceId` after `SemanticBuilder` has
      run.
- [x] Confirm `Semantic` (or `Scoping`) exposes per-symbol declaration
      span — either `symbol_span(SymbolId) -> Span` or via
      `SymbolTable::spans`. Phase 4's binding-bucketing approach needs
      this.
- [x] Confirm `SymbolFlags` variants: `Import`, `Function`,
      `BlockScopedVariable`, `FunctionScopedVariable`, `Class`,
      `TypeAlias`, `Interface`, `Enum`. Adjust phase 4's hoisting map if
      names differ.

### Span ergonomics

- [x] Confirm `oxc_span::Span` has `contains_span(other: Span) -> bool`
      and `start: u32` / `end: u32` field access. Used pervasively by
      phase 4's bucketing.
- [x] Confirm `SourceType::from_path` returns a `SourceType` with
      `jsx=true` for `.tsx`. (Authoritative-by-extension is fine in this
      codebase; no audit needed beyond confirming the helper actually
      sets the flag.)

## Output

- [x] Create `NOTES.swc-to-oxc-migration.01-visitor-spike.md` with one bullet per item
      above, each marked **confirmed** or **corrected: ...**
- [x] Apply every correction back into `PLAN.swc-to-oxc-migration.md` and
      the affected phase TODOs **before** phase 3 begins
- [x] Delete `crates/oxc_spike/`
- [x] `cargo build --workspace` succeeds with the spike removed
