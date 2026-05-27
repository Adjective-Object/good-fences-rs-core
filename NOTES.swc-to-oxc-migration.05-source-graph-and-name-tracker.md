# Notes: phase 05 — source graph & name tracker

## Section 1: Define per-segment binding output

### Key decisions

- **String type**: Used `compact_str::CompactString` (the underlying type behind
  `oxc_str::CompactStr`). `oxc_span::CompactStr` is private at the pinned rev.
  `CompactString` implements `Borrow<str>`, so `AHashMap<CompactString, _>` can
  be queried with `&str` keys directly.

- **VarID dropped**: The old `VarID(Span)` was only used inside
  `ast_name_tracker`'s internal `declare_local` / `VariableScopeError`. The new
  `VariableScope` stores `AHashMap<CompactString, HoistingLevel>` with no span.

- **No VariableScopeError**: The old error type (`DuplicateDeclaration`) was only
  relevant to the SWC visitor that walked child scopes. The new `insert_local`
  is a simple upsert since `oxc_semantic` handles scope rules.

- **Consumer updates**: `source_graph`, `tag_graph`, and `unused_finder` Cargo.toml
  deps updated. `source_graph` and `tag_graph` tests switched from
  `swc_atoms::Atom` to `CompactString`. `unused_finder` only had a Cargo dep,
  no source references.

## Section 2: Implement segment-scoped binding extraction

### Approach

Replaced the `find_names_from_semantic` shim (which returned one flat root-scope
`VariableScope` for every statement) with `variables_for_top_level_statements`,
which computes per-statement scopes in a single pass.

### Algorithm

1. Pre-sort statement spans for binary search (`partition_point`).
2. **Locals**: Iterate root-scope bindings via `scoping.get_bindings()`, bucket
   each into the statement containing its declaration span. O(bindings · log N).
3. **Escaped (resolved)**: For each root binding, iterate its resolved references
   via `scoping.get_resolved_references(symbol_id)`. If a reference falls in a
   different statement than the binding's declaration, mark the name escaped in
   the reference's statement. O(total_resolved_refs · log N).
4. **Escaped (unresolved)**: Iterate `scoping.root_unresolved_references()` (all
   globally unresolved names). For each reference ID, find the containing
   statement and mark escaped. O(total_unresolved_refs · log N).

Total: O(N + M log N) where N = statements, M = symbols + references.

### API notes (oxc at pinned rev)

- `scoping.references` is `pub(crate)` — cannot iterate all references directly.
  Instead used per-symbol `get_resolved_references()` + `root_unresolved_references()`.
- `Reference` has no `name()` method. For resolved refs, the name comes from the
  symbol. For unresolved, it comes from the `root_unresolved_references()` map key.
- `reference_span()` lives on `Semantic`, not `Scoping` (needs AST node access).
- `ReferenceId` is `define_nonmax_u32_index_type!` — could be iterated numerically
  but the per-symbol + unresolved approach is cleaner.
