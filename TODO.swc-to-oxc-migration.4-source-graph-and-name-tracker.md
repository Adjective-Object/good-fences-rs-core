High level goal: delete the `ast_name_tracker` crate. Replace its consumers
(in particular `source_graph`) with data derived from `oxc_semantic`. Swap
`swc_atoms::Atom` for `oxc_span::CompactStr` in long-lived structures.

Depends on phase 3.

## Define the per-segment binding output

`segment_file` currently returns `Vec<Segment>` where each `Segment` carries a
`variables: VariableScope` (locals + escaped) sourced from `ast_name_tracker`.
After this phase, that field is computed from `oxc_semantic` results instead of
a parallel handwritten visitor.

New `Segment.variables` type lives in `ast_segmenter` (replaces
`ast_name_tracker::VariableScope`):

```rust
use oxc_span::CompactStr;
use ahashmap::{AHashMap, AHashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoistingLevel { ImportHoisting, FunctionHoisting, LetConstHoisting }

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariableScope {
    pub local_symbols: AHashMap<CompactStr, HoistingLevel>,
    pub escaped_symbols: AHashSet<CompactStr>,
}

impl VariableScope {
    pub fn get_locals(&self) -> impl Iterator<Item = &CompactStr>;
    pub fn get_locals_with_hoisting(&self) -> impl Iterator<Item = (&CompactStr, HoistingLevel)>;
    pub fn get_escaped_symbols(&self) -> impl Iterator<Item = &CompactStr>;
}
```

- [ ] Move `VariableScope` and `HoistingLevel` from `ast_name_tracker` into `ast_segmenter::variables` (new module)
- [ ] Replace `swc_atoms::Atom` keys with `oxc_span::CompactStr`
- [ ] Drop the `VarID(Span)` field (was unused outside diagnostics)

## Implement segment-scoped binding extraction from `Semantic`

Replace the `ast_name_tracker::visitor::find_names(...)` call in
[ast_segmenter/src/visitor.rs](crates/ast_segmenter/src/visitor.rs) with logic
that builds a `VariableScope` for each top-level statement using
`Semantic.scoping()`:

```rust
fn variables_for_top_level_stmt(
    semantic: &Semantic<'_>,
    stmt_span: Span,
) -> VariableScope {
    let mut scope = VariableScope::default();
    // Bindings declared by `stmt_span`: symbols whose declaration span falls inside stmt_span
    for symbol_id in semantic.scoping().symbol_ids() {
        let decl_span = semantic.symbol_span(symbol_id);
        if stmt_span.contains_span(decl_span) && decl_span_is_top_level(...) {
            let name = CompactStr::from(semantic.symbol_name(symbol_id));
            let hoisting = hoisting_for_flags(semantic.symbol_flags(symbol_id));
            scope.local_symbols.insert(name, hoisting);
        }
    }
    // Escaped: references inside stmt_span whose symbol_id resolves outside stmt_span (or is None)
    for reference in semantic.scoping().references() {
        let ref_span = semantic.reference_span(reference.id());
        if !stmt_span.contains_span(ref_span) { continue; }
        match reference.symbol_id() {
            Some(sym) if stmt_span.contains_span(semantic.symbol_span(sym)) => {}
            _ => { scope.escaped_symbols.insert(CompactStr::from(reference.name())); }
        }
    }
    scope
}
```

(The exact `Semantic` accessors are: `scoping()`, `symbol_name(SymbolId)`,
`symbol_flags(SymbolId)`, `symbols_declared_in_scope(ScopeId)`. Confirm names
against the pinned oxc rev during implementation.)

Mapping from `SymbolFlags` to `HoistingLevel`:

| `SymbolFlags` bit                          | `HoistingLevel`        |
| ------------------------------------------ | ---------------------- |
| `Import`                                   | `ImportHoisting`       |
| `Function`                                 | `FunctionHoisting`     |
| `BlockScopedVariable` / `FunctionScopedVariable` / `Class` / `TypeAlias` / `Interface` / `Enum` | `LetConstHoisting` |

- [ ] Implement `variables_for_top_level_stmt(semantic, stmt_span) -> VariableScope`
- [ ] Implement `hoisting_for_flags(SymbolFlags) -> HoistingLevel`
- [ ] Wire it into `module_item_to_segment(...)` and `module_decl_to_segment(...)`
- [ ] Remove the temporary `ast_name_tracker` shim added in phase 3
- [ ] Add `cargo test -p ast_segmenter` regression coverage: every existing test that asserted on `segment.variables` still passes

## Update `source_graph`

[crates/source_graph/src/lib.rs](crates/source_graph/src/lib.rs)

- [ ] Replace `use ast_name_tracker::visitor::HoistingLevel;` with `use ast_segmenter::variables::HoistingLevel;`
- [ ] Replace `use swc_atoms::Atom;` with `use oxc_span::CompactStr;`
- [ ] Change `name_to_declaring_segments: AHashMap<Atom, Vec<(u32, HoistingLevel)>>` to use `CompactStr`
- [ ] In `resolve_symbol_in_file`, change `Atom::from(name)` lookup to `CompactStr::from(name)` (or take `&str` and use a `Borrow` lookup)
- [ ] Update `build_file` to iterate `seg.variables.get_locals_with_hoisting()` (now yielding `(&CompactStr, HoistingLevel)`)
- [ ] Drop the `swc_atoms` dependency from `source_graph/Cargo.toml`, add `oxc_span`
- [ ] Drop the `swc_common` dev-dependency once the test that uses `BytePos, Span` is updated to use `oxc_span::Span`
- [ ] `cargo test -p source_graph` passes

## Delete `ast_name_tracker`

- [ ] Confirm no remaining callers: `grep -r ast_name_tracker crates/`
- [ ] Remove `crates/ast_name_tracker/` from disk
- [ ] Remove its `members` entry from root `Cargo.toml` (workspace uses `crates/*` glob; nothing to remove unless explicitly listed)
- [ ] `cargo build --workspace` passes
- [ ] `cargo test --workspace` passes
