# SWC → OXC Migration

Migrate `good-fences-rs-core` from the SWC ecosystem to the OXC ecosystem
(parser, AST, visitor, semantic analyzer, diagnostics). The custom node-modules
+ tsconfig resolver is kept (decoupled from `swc_ecma_loader`).

This document records architecture decisions, terminology, dependencies between
phases, and non-goals. Per-phase implementation steps live in
`TODO.swc-to-oxc-migration.<phase>.md`.

## Motivation

- Reduce dependency surface (swc pulls a large transitively-shared crate graph).
- Stop relying on the global `GLOBALS`/`Mark` thread-local model used by
  `swc_ecma_transforms::resolver` for scope resolution.
- Replace the hand-rolled `ast_name_tracker` scope analyzer with
  `oxc_semantic`, which gives first-class symbols, references, and hoisting.
- Remove the nightly `box_patterns` feature gate.

## Source

OXC is pinned to git rev `87f065ebf7cde21d1229322f4c7ee58baca5648e`.

Workspace dependencies (added in Phase 0):

```toml
oxc_allocator   = { git = "https://github.com/oxc-project/oxc", rev = "87f065ebf7cde21d1229322f4c7ee58baca5648e" }
oxc_ast         = { git = "...", rev = "..." }
oxc_ast_visit   = { git = "...", rev = "..." }
oxc_parser      = { git = "...", rev = "..." }
oxc_semantic    = { git = "...", rev = "..." }
oxc_span        = { git = "...", rev = "..." }
oxc_diagnostics = { git = "...", rev = "..." }
```

`oxc_codegen` is not adopted (printing was only used by tests in the
to-be-deleted `swc_utils_print` crate). `oxc_resolver` is not adopted (we keep
the existing custom resolver).

## Core architectural shifts

### Arena-allocated AST

OXC allocates the AST inside an `oxc_allocator::Allocator` (bumpalo arena). All
AST node references, `oxc_span::Atom<'a>` instances, and `oxc_allocator::Vec`s
borrow from that arena. Consequences:

- The parse pipeline must follow an **extract-then-drop** pattern:
  - Create an `Allocator`.
  - Parse + run semantic builder + run visitors that copy owned data out into
    `Segment`/`RawModuleDeps`.
  - Drop the `Allocator`.
- Data structures that cross crate boundaries or outlive a single file's parse
  (e.g. `ast_segmenter::Segment`, `source_graph::SourceGraphFile`,
  `ast_name_tracker::VariableScope`) **must not** hold `Atom<'a>` references.
- `Allocator` is `Send` but not `Sync`. We use a **per-worker thread-local
  arena** held in a `thread_local! { static ARENA: RefCell<Allocator> }` cell.
  Each Rayon worker borrows the arena, parses one file, extracts owned data,
  then calls `Allocator::reset()` (cheap — drops bumpalo chunks back to the
  free list without freeing the backing allocation). This avoids per-call
  allocator construction and keeps the steady-state arena size bounded by the
  largest file each worker has parsed.

  All visitor data that escapes a parse must be owned (no `Atom<'a>` / no
  `&'a` into AST), or the next `reset()` will dangle. The `Segment` /
  `RawModuleDeps` / `VariableScope` types are already shaped this way per the
  rule above.

### Owned identifier type: `CompactStr`

Replace `swc_atoms::Atom` (globally interned, `'static`-ish) with
`oxc_span::CompactStr` (24-byte SSO) wherever the value is stored beyond the
arena lifetime. Public APIs that exposed `swc_atoms::Atom` (notably
`VariableScope::get_locals`) become `CompactStr`-shaped.

> **Spike correction (phase 1):** Identifier `name` fields in this OXC rev are
> `Ident<'a>` (from `oxc_str`), not `Atom<'a>`. `Ident<'a>` implements
> `Deref<Target = str>`, so `.name.as_str()` or `&*node.name` gives a `&str`.
> Use `.to_compact_str()` / `.to_string()` to obtain an owned value that
> outlives the arena.

### Semantic-driven scope analysis

`ast_name_tracker` is **deleted**. `oxc_semantic::SemanticBuilder` provides:

- `Semantic.scoping()` — full scope tree
- `SymbolFlags` — distinguishes function-scoped, block-scoped, function, and
  import bindings (replaces our `HoistingLevel` enum)
- `Reference::symbol_id()` — resolves a name to its binding, or `None` if it is
  an unresolved (global) reference. This replaces the `require_identifiers:
  AHashSet<Id>` trick in `ExportsVisitor::visit_call_expr`.

Mapping from `HoistingLevel` to OXC `SymbolFlags`:

| Old `HoistingLevel`  | New `SymbolFlags`                                        |
| -------------------- | -------------------------------------------------------- |
| `ImportHoisting`     | `SymbolFlags::Import`                                    |
| `FunctionHoisting`   | `SymbolFlags::Function`                                  |
| `LetConstHoisting`   | `SymbolFlags::BlockScopedVariable` (let/const) or `FunctionScopedVariable` (var) |

> **Spike correction (phase 1):** `SymbolFlags::Enum` is a composite alias
> (`ConstEnum | RegularEnum`), not a single bit. Use
> `flags.intersects(SymbolFlags::Enum)` to test for enum symbols. Also, there
> is **no** plain `references()` iterator on `Scoping`; to iterate all
> references use `scoping.symbol_ids()` + `scoping.get_resolved_references(id)`,
> or walk AST nodes via `Semantic::nodes().iter()`.

### Spans

| swc                                  | oxc                                  |
| ------------------------------------ | ------------------------------------ |
| `swc_common::Span { lo, hi, ctxt }`  | `oxc_span::Span { start: u32, end: u32 }` (`Copy`) |
| `swc_common::BytePos(u32)`           | bare `u32`                           |
| `swc_common::source_map::SmallPos::to_u32()` | field access on `Span`        |
| `span.contains(other)` (swc)         | `span.contains_inclusive(other)` (oxc — **no** `contains_span`) |

`Segment.span`, `TaggedSymbol.span`, `ReExportedSymbol.span`,
`ExportedSymbolMetadata.span` all become `oxc_span::Span`.

### Comments

OXC stores `Vec<Comment>` directly on `Program` in source order (sorted by
`span.end`); there is no `Comments::get_leading(BytePos)`. We do **not**
introduce a wrapper type — visitors take `comments: &[Comment]` directly
(`&program.comments`) and resolve "comments leading statement at offset N" by
binary-searching the slice for the contiguous run of comments whose
`span.end <= N` and that aren't separated from N by another statement.

A small free function lives in `oxc_utils_parse`:

```rust
pub fn leading_comments_at(comments: &[Comment], statement_start: u32) -> &[Comment];
```

`SymbolTags::from_comments(comments: impl Comments, lo: BytePos)` becomes
`SymbolTags::from_comments(comments: &[Comment], lo: u32)` and internally
calls `leading_comments_at`.

### Diagnostics

`swc_common::errors::Handler` is replaced with `oxc_diagnostics::OxcDiagnostic`
+ `NamedSource`. The `logger_srcfile` crate adopts `NamedSource` for
file-aware rendering. `WrapFileLogger` holds **only** the `NamedSource<String>`
(no separate `source` field); the source text is retrieved via
`NamedSource::inner()` when needed, and `line_starts: Vec<u32>` is computed
lazily on first error/warning. Parser errors come back in
`ParserReturn.errors: Vec<OxcDiagnostic>` and are joined as message strings
(current behavior) or rendered with `miette` (richer output).

### Resolver

`import_resolver/src/swc_resolver/` is renamed to
`import_resolver/src/node_resolver/`. The two swc surfaces it consumes are
replaced by a local trait:

```rust
use oxc_span::CompactStr;

pub struct Resolution { pub path: PathBuf, pub slug: Option<CompactStr> }

pub trait PathResolver: Send + Sync {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution>;
}
```

`swc_common::FileName::Real(p)` use sites become `&Path` directly. The other
`FileName` variants (`Url`, `Anon`, `Custom`, `Macros`, etc.) are not used by
any good-fences code path — every existing `match base { FileName::Real(p) =>
p, _ => bail!(...) }` branch unconditionally errored. Phase 2 simply drops the
match and uses the `&Path` directly. Internal caching (`ftree_cache`,
`MonorepoResolver` ouroboros wrapper, `mark_dirty_root`) is preserved as-is.

`NODE_BUILTINS` and `TargetEnv` are copied locally (each is a small constant
list + enum).

## Terminology

- **Phase** — a TODO file. Each phase compiles, tests pass, and can be merged
  independently.
- **swc stack** — the existing crates (`swc_common`, `swc_ecma_*`, etc.).
- **oxc stack** — the replacement (`oxc_allocator`, `oxc_ast`, `oxc_parser`,
  `oxc_semantic`, `oxc_ast_visit`, `oxc_span`, `oxc_diagnostics`).
- **Extract-then-drop** — the parse pattern described above.

## Phase dependencies

```
0-scaffolding
   └─→ 0.5-visitor-spike      (validates oxc_ast / oxc_ast_visit shape before 3+)
          ├─→ 1-logger-srcfile       (independent leaf)
          ├─→ 2-resolver-decoupling  (independent of AST work)
          └─→ 3-ast-segmenter
                 └─→ 4-source-graph-and-name-tracker
                        └─→ 5-unused-finder ←── (2-resolver-decoupling)
                               └─→ 6-good-fences-get-imports
                                      └─→ 7-tag-graph
                                             └─→ 8-cleanup
```

Phases 1 and 2 are independent of the rest and can land first (but after 0.5
so the visitor shape is locked in).

Both stacks coexist between Phase 0 and Phase 8. Phase 8 removes the swc
workspace deps and the `box_patterns` feature.

## Non-goals

- Adopting `oxc_resolver`. The custom resolver has tsconfig-paths handling,
  package.json browser/exports semantics, ftree-backed caching, and a
  `mark_dirty_root` API used by `unused_finder`'s incremental path. Replacing
  it is out of scope.
- Adopting `oxc_codegen` or any AST printer. The only printer call site
  (`swc_utils_print::normalise_src`) is test-only and is deleted in Phase 8.
- Replacing `ahashmap::AHashMap` with `oxc_data_structures::FxHashMap`.

### Expected breakage

- The napi crates' wire format **will change** for any exported type that
  contained `swc_atoms::Atom`-shaped names (notably anything reachable from
  `VariableScope` or `Segment` if those leak through `unused_finder_napi` /
  `good_fences_napi`). Atoms become plain JS strings on the boundary. This is
  acceptable; consumers of the napi packages must rebuild and re-deserialize
  any cached output. Phase 4 audits the napi surface for atom-shaped exports.

## Cross-cutting test gates

After each phase:

- `cargo test --workspace`
- The NAPI integration test in `__test__/index.spec.mjs`
- `cargo build -p good_fences_napi -p unused_finder_napi` (verify napi crates
  still link)

After Phase 8:

- Run the full benchmarks (see crate-level `cargo bench` targets) on the
  existing test corpus. (No formal baseline is captured; eyeball for order-
  of-magnitude regressions.)
