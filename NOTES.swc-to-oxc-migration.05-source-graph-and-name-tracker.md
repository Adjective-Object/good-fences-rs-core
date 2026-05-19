# Phase 5 notes: source-graph and name-tracker

## `oxc_span::CompactStr` is not publicly re-exported

Despite the PLAN referencing `oxc_span::CompactStr`, this type is not actually
re-exported from `oxc_span`. It lives in `oxc_str::CompactStr` (a thin wrapper
around `compact_str::CompactString`). We use `compact_str::CompactString`
directly (already a transitive dep at v0.9.0). `CompactString: Borrow<str>` so
`AHashMap<CompactString, _>.get(name: &str)` works without any conversion.

## `variables_for_top_level_statements` algorithm

Single-pass O(N + M log N):

1. Build `stmt_spans: Vec<(u32, u32)>` (sorted because body order equals span order).
2. Walk only `scoping.iter_bindings_in(root_scope_id())` for locals — avoids
   picking up inner-scope declarations.
3. Walk `scoping.symbol_ids()` + `scoping.get_resolved_references(sym_id)` for
   escaped resolved symbols, and `scoping.root_unresolved_references_ids()` for
   unresolved (global) escaped symbols.
4. Binary-search `find_stmt(offset)` via `partition_point` to map each span to
   its statement index.

## Type-only references must be excluded from escaped symbols

OXC resolves type-parameter constraints (`T extends MyInterface`) as references
with `ReferenceFlags::Type`. Including these in escaped symbols causes
type-only names to get `FROM_ENTRY` propagation in `tag_graph` — incorrectly
treating them as runtime value uses.

Fix: check `reference.flags().is_type_only()` and `continue` in the escaped
symbol loops (both resolved and unresolved). `is_type()` (bit set test) would
also skip mixed value+type references; we use `is_type_only()` to be
conservative (only skip pure type refs, preserve mixed ones).

## `parse_file` now always uses TypeScript mode

`SourceType::from_path("file.js")` returns JavaScript mode, which rejects
TypeScript syntax (`export type`, interfaces, etc.) common in `.js` files in
this codebase. Changed to: use `TS` for everything except `.jsx`/`.tsx` which
use `TSX` mode. This is a phase-4 oversight that surfaced during phase-5
testing.

## `ExportNamedDeclaration`/`ExportDefaultDeclaration` need both visitors

The `ExportsVisitor` does not descend into statement bodies to find `import()`
calls. Exports like `export const x = import('./lazy')` were losing their
dynamic import. Fix: call `find_imports_and_requires` on the same node and
merge the resulting `dynamic_imports`/`requires` into the segment.

## napi wire-format audit (phase 5)

Grep of `crates/unused_finder_napi/` and `crates/good_fences_napi/` found no
re-exports of `VariableScope`, `Segment`, or any type that previously held
`swc_atoms::Atom`. These types are internal to the Rust computation. No napi
wire-format change for external consumers.

## Statement index alignment

`variables_for_top_level_statements` returns a `Vec<VariableScope>` whose index
matches `program.body[i]`. In `segment_file`, statements must be enumerated
(`body.iter().enumerate()`) so that `variables[i]` lines up with the i-th
top-level statement. The body is pre-filtered to only module declarations; the
variables vec covers all statements, so the index into `variables` uses the
original unfiltered position.

## NAPI integration tests pass

`yarn test` (project uses yarn, not pnpm) was run after `yarn build:debug`. All
3 integration tests passed. No `.d.ts` regeneration was needed — the napi
wire-format did not change (no `Atom`-shaped types exposed through the napi
boundary).

## Remaining follow-up (not blocking phase 5)

- `segment_graph.rs` still uses `swc_atoms::Atom` for `Name` — deferred to
  phase 8/9 cleanup.
- `repo-health` Dockerfile version mismatch is pre-existing and unrelated.
