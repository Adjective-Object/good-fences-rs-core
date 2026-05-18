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
[ast_segmenter/src/visitor.rs](crates/ast_segmenter/src/visitor.rs) with a
**single-pass bucketed** computation. For a file with N top-level statements
and M total symbols/references, this is O(N + M log N) instead of the
O(N·M) loop the original draft sketched.

```rust
/// Compute `VariableScope` for every top-level `Statement` in one pass.
/// Returned vec has one entry per `program.body[i]` in order.
fn variables_for_top_level_statements<'a>(
    semantic: &Semantic<'a>,
    body: &[Statement<'a>],
) -> Vec<VariableScope> {
    // Pre-sort statement spans for binary search.
    let stmt_spans: Vec<(u32, u32)> =
        body.iter().map(|s| (s.span().start, s.span().end)).collect();
    let mut out: Vec<VariableScope> = (0..body.len()).map(|_| VariableScope::default()).collect();

    let find_stmt = |offset: u32| -> Option<usize> {
        // partition_point on stmt_spans by `end`; check that `start <= offset < end`.
        let i = stmt_spans.partition_point(|(_, end)| *end <= offset);
        stmt_spans.get(i).and_then(|(start, end)| (offset >= *start && offset < *end).then_some(i))
    };

    // Bindings: only walk the program scope (top-level decls).
    let program_scope = semantic.scoping().root_scope_id();
    for symbol_id in semantic.scoping().get_bindings(program_scope).values().copied() {
        let decl_span = semantic.symbol_span(symbol_id);
        let Some(stmt_idx) = find_stmt(decl_span.start) else { continue };
        let name = CompactStr::from(semantic.symbol_name(symbol_id));
        let hoisting = hoisting_for_flags(semantic.symbol_flags(symbol_id));
        out[stmt_idx].local_symbols.insert(name, hoisting);
    }

    // Escaped: iterate references once; classify by containing statement.
    for reference in semantic.scoping().references() {
        let ref_span = semantic.reference_span(reference.id());
        let Some(stmt_idx) = find_stmt(ref_span.start) else { continue };
        let resolves_inside = match reference.symbol_id() {
            Some(sym) => {
                let sym_span = semantic.symbol_span(sym);
                let (s, e) = stmt_spans[stmt_idx];
                sym_span.start >= s && sym_span.end <= e
            }
            None => false,
        };
        if !resolves_inside {
            out[stmt_idx].escaped_symbols.insert(CompactStr::from(reference.name()));
        }
    }

    out
}
```

(The exact `Semantic` accessors — `scoping()`, `root_scope_id()`,
`get_bindings()`, `symbol_name`, `symbol_span`, `symbol_flags`,
`references()`, `reference_span`, `Reference::id`/`symbol_id`/`name` — are
confirmed in phase 0.5.)

Mapping from `SymbolFlags` to `HoistingLevel`:

| `SymbolFlags` bit                          | `HoistingLevel`        |
| ------------------------------------------ | ---------------------- |
| `Import`                                   | `ImportHoisting`       |
| `Function`                                 | `FunctionHoisting`     |
| `BlockScopedVariable` / `FunctionScopedVariable` / `Class` / `TypeAlias` / `Interface` / `Enum` | `LetConstHoisting` |

- [ ] Implement `variables_for_top_level_statements(semantic, body) -> Vec<VariableScope>` as above
- [ ] Implement `hoisting_for_flags(SymbolFlags) -> HoistingLevel`
- [ ] Wire it into `segment_file(...)`: compute the vec once at the top, index by statement position
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

## Audit napi surface for atom-shaped exports

The napi wire format will change for any exported type that contained
`swc_atoms::Atom`-shaped names (atoms become plain JS strings on the
boundary). This is acceptable per the plan; the audit just records what
changes so consumers know to rebuild.

- [ ] Grep `crates/unused_finder_napi/` and `crates/good_fences_napi/` for re-exports of `VariableScope`, `Segment`, `RawModuleDeps`, or any other type that previously held `Atom`
- [ ] List each napi-exported type whose wire format changes in a new section of `NOTES.swc-to-oxc-migration.md` titled `## napi wire-format changes (phase 4)`
- [ ] `pnpm test` (NAPI integration tests) passes after re-generating any `.d.ts` files
