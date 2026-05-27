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
