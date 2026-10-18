# Incremental JavaScript bundler plan

## Scope and status

The planning increment delivered **P0**: this grounded plan, a small portable
esbuild/rspack fixture corpus, and a baseline execution driver. Subsequent
increments follow P1–P9 toward the full bundler epic. P0 is not a bundler
implementation. Autonomous implementation now proceeds in narrow linear
commits: P0–P7 are verified for the documented synchronous Node ESM/CJS target;
P8.1 source dynamic loading and P8.2 native TLA scheduling are verified;
P9 remains pending.
This bounded phase completes P8.1–P8.2 and stops before P9.
The checklist in
`TODO.incremental-bundler.md` is the execution ledger.

P7.1 adopts Node createRequire + synchronous source hooks for opaque CJS,
preserving native early cache insertion, partial cycles, wrapper aliases/this,
replacement values and retry after factory throws. CJS/ESM proxy interop follows
in P7.2; no hand-written competing require cache is introduced.
P7.2 supplies native ESM proxies using reverse typed requests/checked original
barrel identities: named properties snapshot once, default captures full require
result, __esModule is ignored. P7.3 adds conservative runtime-key namespaces:
keys snapshot at initialization, reads forward live to the captured exports.
Format now survives owned graph/cache facts (schema 4/extractor 5), including
package-commonjs `.js` files. No source assignment guessing defines exports.
P7.3 integrates initializer/default, runtime-key namespace and named snapshot nodes
into exact-root partitioning, manifests and JSON source payloads. No facade
payload bypass, node duplication or root-set coalescing is introduced. Native
require/ESM owns shared identities and legal mixed cycles; evaluating-ESM require
cycles retain native errors. Eight portable sync fixtures/eleven executions and
same-process concurrent-entry/cross-file-cycle observations have bundled evidence.
P8 now connects source dynamic imports and native TLA scheduling (below);
publication/invalidation remains P9.

P1.4 adds `ParsedModule::facts` (`ast_segmenter::owned`): owned lexical
binding/declaration/reference records, import aliases and export mappings,
individual dependency sites, directives and hashbang. IDs are source-local
byte offsets, not OXC IDs. P1.5 adds `ModuleBody::CommonJs`, one opaque
executable body, wrapper-reference classification and checked source-local
support policy. The [crate support matrix](crates/ast_segmenter/README.md#source-local-support-matrix)
distinguishes syntax support/rejection from deferred target resolution and
runtime/transform gates. The legacy maps below remain lossy adapters and
must not be used as the occurrence IR.

This plan was grounded by reading the current sources, not by assuming checked
historical TODOs prove correctness. No Rust tests were run for this documentation
work. `NOTES.incremental-bundler.md` defines phase-0 ownership and interchange.
Both completed upstream research notes are incorporated below. Integration
verified all **12 imported runtime contracts** (six esbuild and six rspack,
15 entrypoint executions), all **31 harness regressions**, and
`./scripts/spot-check`. The scoped `.gitignore` exception includes every fixture
`index.js`; integration reports no ignored files remaining under `fixtures`.
An isolated Node 22 CI job now runs the fixture checks. P0 is complete.
These results establish a native Node baseline and the stated spot-check,
not bundler parity or a passing full workspace test suite.

Persistent cross-cutting architecture decisions belong concisely in root
`AGENTS.md`, and crate-specific decisions in crate-root `AGENTS.md`, when adopted.
Work-in-progress findings belong in `NOTES.incremental-bundler*.md`. This plan
describes contracts and their implementation checkpoints; it is not evidence of
emitted runtime correctness.

## 1. P0 inventory: reusable pieces and compatibility gaps

This inventory records the original legacy-path baseline. P1–P4 checkpoints
below supersede the listed gaps for the separate owned bundler path.

| Evidence | Exists now | Not a bundler guarantee |
| --- | --- | --- |
| `crates/ast_segmenter/src/visitor.rs`: `segment_file`, `variables_for_top_level_statements`, `statement_to_segment`; `parse.rs`: `parse_module` | One owned `Segment` per accepted ESM top-level statement; one opaque body for CJS. `parse_module` checks parser/semantic diagnostics and source-local support policy, and extracts occurrence facts alongside legacy statement buckets. | Legacy `segment_file` can log and skip rejected statements. `try_segment_file` checks legacy statement coverage, not the full bundler subset; use `parse_module` for owned bundler facts. Parse records are not transformed output, linked modules or runtime support. |
| `segment_info.rs`: `Segment`; `variables.rs`: `VariableScope` | Owned module dependencies, name/hoisting sets, and an OXC statement `Span`; arena lifetime does not escape. | No serialized IR, module classification, effect summary, reference occurrence ranges, stable lexical binding IDs, or rewrite recipe. Names alone cannot support renaming or live bindings. |
| `raw_module_deps.rs`: `RawModuleDeps`, `TaggedSymbol`, `SymbolTags`; `visitors/import_export_statement.rs`: `ExportsVisitor` | Static imports, local exports, re-exports, dynamic imports, require paths, execution-only imports, and type/allow-unused tags. | Maps/sets collapse occurrences. Static import records do not provide a full local-binding-to-import-target rewrite table. Do not confuse comments allowing unused exports with purity annotations. |
| `visitors/import_require_expr.rs`: `ImportsAndRequires`, `is_global_require`, `scan_import_expr`, `scan_call_expr` | Recognizes string-literal `require` and `import`; rejects shadowed `require` as a module dependency using semantic binding resolution; recognizes a `.then` destructuring pattern. | No source-site boundary IDs; nonliteral paths silently produce no dependency. `.then` name narrowing must be re-proven for rest/computed/default patterns and repeated sites before tree shaking; it is not an execution model. |
| `crates/oxc_utils_parse/src/lib.rs`: `parse_file`, `parse_bundler_file` | Unchanged tracker compatibility mode parses TS, or TSX for JSX/TSX extensions. P1.3 adds extension-sensitive JS/TS/JSX parsing and explicit package format; `.mjs`/`.mts` and `.cjs`/`.cts` override it. | The tracker helper still does not classify `.cjs` as CJS. Bundler package discovery belongs to callers; parser validation alone does not replace semantic diagnostics, TS/JSX transformation or runtime policy. |
| `crates/unused_finder/src/parse/exports_visitor_runner.rs`: `get_file_segments`, `PARSE_ARENA` | Reads source; reports parser errors; extracts owned data; resets a worker-local OXC arena. | Legacy tracker still skips an auto-generated-file marker and discards semantic-builder diagnostics. P1.3's separate `parse_module` does neither. No disk parse cache. |
| `crates/segment_cache`: `CacheRecord`, `load_or_parse` | P2 deterministic bounded CBOR persistence of existing owned facts/body and ordered statement ranges; source/config identity, atomic sibling sidecars, explicit cache outcomes and opt-out. | Not automatically enabled for the legacy tracker; no persisted usage tags, resolved paths or derived effects. Source-adjacent IO is a snapshot API, not a cross-file transaction with external editors. |
| `crates/source_graph/src/lib.rs`: `SourceGraph`, `SourceFileInput`, `resolve_import_across_files`, `patch_file` | Owns segments and path indexes; follows named/star re-exports with cycle detection; patches/rebuilds one file and clears all re-export cache entries. | No durable IDs, deletion API, typed adjacency, or incremental consumer invalidation. `patch_file` must not accept a mismatched path/input path as a rename. `RefCell` cache is not a ready concurrent linker cache. |
| Same file: `resolve_symbol_in_file`, `resolve_import_inner`, `build_file` | Name/hoisting lookup and direct/named/star export search. | Let/const references before declaration return no target, although they still bind lexically and may trigger TDZ; `hoisting_for_flags` puts `var` in the fallback category. Namespace-under-name re-exports are explicitly deferred. Star search does not exclude `default` or diagnose ambiguity. Cycle termination is not ESM evaluation correctness. |
| `crates/source_graph/src/edge.rs`: `SegmentEdge` | A public edge struct containing `from`, `to`, `is_type_only`. | The existence of this struct does not mean graph adjacency uses it. It has neither sync/async kind nor occurrence, evaluation, or boundary metadata. |
| `crates/tag_graph/src/lib.rs`: `TagGraph`, `propagate_tags_to_used`, `propagate_tags_to_users`, private `build_reverse_edges` | Usage-specific BFS; type-only filtering; reverse traversal rebuilt from source data; explicit side-effect-only import traversal. | Dynamic imports and static imports are traversed alike. `UsedTag` is four usage flags, not an arbitrary root-identity set. Reverse edges are private, reconstructed separately, and do not constitute a shared occurrence-aware index. |
| `crates/unused_finder/src/walked_file.rs`: `ResolvedSourceFile::specifier_to_resolved`; `unused_finder.rs`: `to_source_file_input`, `build_result_graph` | Adapter from unused-finder's resolved files into the source graph and report structures. | Resolution lives in an application pipeline, not a bundler-neutral persisted parse product. File-level and segment-level type hierarchies still coexist in `parse/data.rs`. |
| `crates/unused_finder/src/report.rs`: `compute_unused_segments`; `crates/ast_segmenter/src/segment_graph.rs`: `SegmentGraph` | A second graph implementation still drives unused-segment reporting. It chains non-hoisted statements and propagates a `TagSet`. P1.2 preserves the resolver mapping in results and connects execution-only imports/requires to every target statement. | This remains conservative compatibility analysis, not effect analysis or evaluation scheduling. Two graph implementations still need convergence in P3.4. |

Additional compatibility gates:

- `TODO.segment-aware-migration.md` and `TODO.source-graph.md` contain completed
  migration checklists, but the above parallel graph/type paths still exist.
- P1.1 reproduced `unused_finder::test::test_typeonly_interface_allowed`
  failing because `[TypeOnly]` became `[Entry, TypeOnly]`, then fixed lost
  type-reference metadata and intra-file tag traversal without changing the
  expected report. See the implementation checkpoint in NOTES for verification;
  this closes that compatibility gate, not the remaining graph/report gaps.
- P1.2 pins existing symbol/segment report ranges as zero-based, end-exclusive
  UTF-8 byte offsets, including offset zero and multibyte source. Corrected stale
  SWC documentation and test slicing without shifting existing OXC output.
- Workspace OXC components are pinned together at
  `87f065ebf7cde21d1229322f4c7ee58baca5648e` in `Cargo.toml`.
  There is no workspace `oxc_codegen`/transformer dependency. P2 adds the CBOR
  codec separately from OXC. Check APIs at that revision, not current upstream docs.
  Workspace serde is constrained to `>=1.0.117, <1.0.219`; `segment_cache`
  uses serde_cbor 0.11.2 with locked serde 1.0.218 and SHA-256 0.10.9.

### Pinned esbuild grounding

The completed `NOTES.incremental-bundler.esbuild.md` records inspection at
[`f6058f8364fe7ab91ca57a83e02577ed74c9cae4`](https://github.com/evanw/esbuild/tree/f6058f8364fe7ab91ca57a83e02577ed74c9cae4)
with MIT licensing verified by the importer. These are upstream design
references, not claims that our proposed implementation already matches it:

- [Top-level parts](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_ast/js_ast.go#L1740-L1791)
  carry declaration/use facts, import records, dependencies and removability.
  [Link ordering](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L227-L350)
  separates import/export matching, liveness, reachability, chunks and generation.
  This supports shared parse/structure layers, not reuse of usage-report tags
  as executable semantics.
- [File-level effects](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/graph/input.go#L49-L80)
  default conservatively; per-part removability is separate. Our P4 preserves
  both module-evaluation demand and statement effects. Package `sideEffects`
  metadata is a later explicit policy, not something native fixtures validate.
- [PURE argument preservation](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_ast/js_ast_helpers.go#L812-L824)
  motivates effect-preserving rewrites instead of declaring an annotated
  statement wholly pure.
- [Liveness then reachability](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3148-L3218)
  and [entry-bitset chunk grouping](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3875-L3992)
  ground exact-set grouping. **esbuild groups live files here**, while this plan
  deliberately proposes segment-level groups and per-import-site root identity.
  Do not describe either difference as already supplied by upstream.
- [Initialization and interop helpers](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/runtime/runtime.go#L170-L269)
  support cycle-safe CJS allocation, getter forwarding and mode-sensitive
  default interop. The note identifies different ESM/CJS failure-cache behavior;
  our runtime still needs a chosen, tested contract.
- [Parse cache](https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/cache/cache_ast.go#L152-L189)
  uses exact source/options equality. Clone/recompute mutable link state rather
  than writing linker-policy conclusions into reusable source parse facts.

The importer reports six standalone cases: `shared-entrypoint-identity`,
`dynamic-import-tla-order`, `esm-sync-async`, `cjs-sync-async-once`,
`unused-import-side-effects`, and `reexport-cycle`. Its native validation
reports six manifests/nine Node invocations with matching output; all six also
passed the integrated shared runner. The shared-entrypoint case includes
an explicit `node.js` driver importing the entries together, so its identity
assertion does exercise one process even though manifest entries are otherwise
separate processes. The CJS once-only log is a documented added observation.
TLA began as an explicit capability gate, not a requirement silently waived by
a passing Node baseline; P8.2 now executes its generated artifacts. None of these cases
validates PURE hints, chunk layout, or tree-shaking optimizations by itself.

### Pinned rspack grounding

The completed `NOTES.incremental-bundler.rspack.md` records inspection of
[`web-infra-dev/rspack` at `96dcd72db4d25a89c37d22d514285ab82fc46a3d`](https://github.com/web-infra-dev/rspack/tree/96dcd72db4d25a89c37d22d514285ab82fc46a3d).
The importer also compared inherited fixtures with webpack at
`e12e1224c2824c0cc78d6f3faf4cfa47926aa3cf`; that is a comparison pin, not a
claimed historical import commit. Preserved rspack/third-party/webpack license
notices and the corpus `NOTICE` document the independently applicable MIT
attribution. The integration ignore-rule fix supersedes the handoff note's
earlier warning about hidden `index.js` files.

- [Normal-case processing](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/packages/rspack-test-tools/src/case/normal.ts#L36-L70)
  and its
  [default options](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/packages/rspack-test-tools/src/case/normal.ts#L108-L211)
  compile under an upstream harness with async-node, analysis and resolution
  settings. Portable adaptations therefore retain observations, not the
  assumption that copied upstream entrypoints run unchanged.
- [The CJS self-cycle](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/runtime/circular-dependencies/circular.js#L1-L2)
  reassigns `module.exports` before requiring itself. It grounds early cache
  insertion and returning the current replacement value, but does not cover
  arbitrary multi-module SCCs or every partial-export case.
- [Default CJS import](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/interop/exports/index.js#L1-L4)
  and [named requests through a facade](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-commonjs/index.js#L25-L30)
  ground separate request/export models. Node's acceptance of static
  `exports.a`/`exports.b` does not decide arbitrary dynamic names, `__esModule`,
  or require-of-ESM interop.
- [ESM live-binding mutation](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-spec/index.js#L8-L12)
  observes updated exported storage, not a snapshot import value.
  [An import after a logging statement](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-spec/order-c.js#L1-L4)
  still evaluates its dependency first. Together these require binding storage
  and module initialization order beyond a statement-level dependency graph.
- [Dynamic CJS import](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/chunks/import/index.js#L1-L11)
  supplies a promise/namespace observation, not a chunk layout contract. The
  adapted trace adds an explicitly documented asynchronous-order observation.

All six integrated cases passed: `cjs-default-export`, `cjs-named-reexport`,
`cjs-self-cycle`, `dynamic-import-cjs`, `esm-live-binding`, and `esm-effect-order`.
Exact namespace metadata keys are intentionally excluded. Eval, transpiled
live-binding cases, additional cycles, bundler used-export globals and
magic-comment behavior remain future coverage. Upstream
[`await require` of an async module](https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/async-modules/double-import/index.js#L1-L4)
was deliberately not recast as a native passing contract; TLA/require policy
remains a later implementation decision, not a P0 blocker.

## 2. Proposed layering and invariants

Names below identify responsibilities; new package names are provisional.
Prefer extracting reusable libraries over teaching `unused_finder` to bundle.

1. **Parse/extraction** (`ast_segmenter` + `oxc_utils_parse`): source-local owned
   records, diagnostics, lexical bindings, dependency occurrences, and spans.
   Preserve an adapter with today's unused-tracker parser policy.
2. **Persistence** (a small `segment_cache` library if justified): versioned DTO
   and sidecar IO independent of resolver, effects policy, and graph consumers.
3. **Resolution/structure** (`import_resolver` adapter + `source_graph`):
   resolved module identities, export bindings, typed forward/reverse adjacency,
   and revisions. Read `crates/import_resolver/AGENTS.md` before modifying it.
4. **Analysis consumers**: existing `tag_graph` usage analysis; separate
   bundler effect/reachability/chunk analyses. Shared structure must not encode
   `allow_unused_types`, entry-package rules, or bundler runtime policy.
5. **Link/emit** (proposed bundler library): module instantiation/evaluation,
   interop, lexical source rewriting, chunks, and output manifests.
6. **Application**: thin orchestration/CLI only after library contracts work.
   A future `-cli` package exports both clap definition/library and binary.

### Parse product and `<source>.seg.cbor`

- Cache an owned `ParsedModule`-style DTO: source length/hash, local module
  syntax facts, ordered segments, directives/hashbang, declarations and lexical
  references, import/export occurrences with raw specifiers, local binding IDs,
  type-only metadata, source ranges, and syntax-local effect facts.
- Ranges are checked UTF-8 byte offsets `[start, end)` into the exact hashed
  source. Whole statements need not partition comments/whitespace. Imports,
  identifier references, expressions, and edit sites need their own ranges.
- CJS has **one opaque executable body segment** spanning its body, with
  separately extracted dependency occurrences. Do not tree-shake CJS internals.
  Synthetic ESM proxies are linker nodes, not slices falsely attributed to it.
- Separate syntactic module facts from environment-sensitive interpretation:
  a package `type` change may alter classification/parser mode and force a
  reparse; package `exports`/resolution condition changes usually relink only.
- Header: magic, schema and extractor versions, exact OXC/parser revision,
  source digest/length, parser options/language mode/target-affecting settings,
  and checks for structural validity. Hash identity is authoritative; mtime is
  only an optional fast precheck. Sort unordered collections when encoding.
- Do not persist arena IDs, absolute resolved paths, process-local `SegmentKey`
  file numbers, usage tags, chunk membership, or linker-config-derived effects
  as unconditional source truth.
- Write a unique sibling staging file and atomically replace the sidecar;
  define Windows replacement behavior and concurrency ownership. Failed writes
  cannot replace a valid cache. Unwritable sources fall back to uncached parsing
  with a documented diagnostic policy. Exclude sidecars from source watching.
- Unknown schema, corruption, truncation, impossible ranges, oversized
  allocations, source mismatch, and stale parser config are safe misses, not
  successful parses. A clean parse failure must not resurrect old output.
- Relocation reuses source-local bytes only if effective parse policy agrees;
  re-resolve environment-sensitive data under the new root. Source deletion,
  orphan sidecars, symlinks/casing policy, and concurrent source modification
  during hashing/parsing/writing need explicit tests.

### Graph and linker contract

- Edge direction is **consumer → dependency**; store the reverse index from the
  same authoritative edge collection. Keep dependency kind (local reference,
  static import/re-export, execution, require, dynamic import), sync/async
  activation, type-only status, source occurrence, and imported/local names.
- Syntax locality and evaluation timing are different: a literal require inside
  a function is a synchronous dependency *when called*, not permission to
  eagerly execute it. A function body's references do not make function
  declaration evaluation effectful.
- Separate symbol retention from module evaluation. Importing one named binding
  still evaluates the target's retained top-level effects and transitive static
  module dependencies. An empty module may still need a module identity/node.
- Resolve local references by semantic binding identity, including forward
  lexical references; TDZ is an evaluation constraint, not “unresolved name.”
  Export lookup must support aliases/default/namespace exports, explicit export
  precedence, star default exclusion, ambiguous stars, cycles, and errors for
  missing runtime bindings. Type-only paths never become runtime dependencies.
- Maintain graph revision/generational keys so replacing a file cannot reuse
  stale tags/proxies/edges by coincidentally reusing a segment index.

### Conservative effects, then annotations and patterns

Start esbuild-inspired, **not claiming esbuild-equivalent behavior**: ordinary
function creation is pure; unknown expressions may effect or throw. Retain
unknown calls/new expressions, property reads (getters/proxies), coercions,
assignments, spreads/iteration, class heritage/computed keys/static blocks,
and unresolved global reads conservatively. Distinguish declaration evaluation
from body invocation; default arguments evaluate at invocation.

`/* @__PURE__ */` / `/* #__PURE__ */` support comes later with precise attachment.
A pure call annotation is not permission to drop effectful callee/receiver/
argument evaluation. Tests must cover `/* @__PURE__ */ f(sideEffect())`, spreads,
optional chaining, and result-used calls.

A linker-supplied known-effectful expression DSL, e.g. `*.appendChild(...)`,
must be parsed and matched structurally, never executed as JavaScript or matched
by raw source substring. Specify wildcard/computed/optional-chain/alias behavior.
Initially patterns can only add effectfulness; explicit effectful matches win
over purity hints. Invalid/unsupported patterns are diagnostics. Pattern changes
invalidate effect/reachability/chunk/output results, not the source-local parse
cache. Unknown nonmatches remain conservative.

### Exact chunk identity

Let roots be tagged identities `Entry(entry_id)` and
`Async(import_site_id)`; use individual source occurrences rather than raw path
strings so repeated imports of one target remain distinguishable. Each retained
dynamic site activates its own root. Discover nested sites to a fixed point;
unreachable sites do not create chunks.

For each root, flood-fill consumer-to-dependency **synchronous** edges, including
required evaluation/effect and synthetic proxy dependencies. Do not carry the
parent root across an asynchronous edge. On every retained segment `s`, record
the exact canonical set `R(s)` of roots that reach it. Two segments belong to
the same logical chunk iff their nonempty sets are equal. An empty set is dead.
Synchronous SCCs must receive identical root sets by fixed-point propagation.

Entrypoint wrappers and dynamic-import facades can be separate output artifacts;
they must not alter logical chunk equivalence. A module reached synchronously
and asynchronously is still evaluated once. Physical loading dependencies,
evaluation scheduling, and exported namespaces are separate from set membership.
Do not execute a dynamic-only branch merely because a different segment in its
chunk was loaded. **No merging, hoisting, or heuristic chunk coalescing** in the
initial implementation. Stable root-set identity and content-addressed output
filename are separate concepts.

### Emission and runtime semantics

- Emit real ESM for ESM modules; preserve module instantiation before evaluation,
  export live bindings, lexical TDZ, declaration hoisting, execution order, and
  SCC behavior. Source rewriting/code generation is required: concatenating
  byte slices or copying `export const alias = importedValue` cannot supply
  general ESM semantics.
- Choose a pinned-OXC AST printer/transform path versus validated source edits
  before emitting. Both require scope-aware binding renames, shorthand handling,
  nonoverlapping edits, import/export rewrites, directives/hashbang preservation,
  syntax validation, and source-map strategy. Reparse hashed source on a cache
  hit if the chosen emitter needs AST information not stored in the DTO.
- CJS runtime owns a registry of factories and a module cache shared across
  all chunks in one runtime instance. Insert the module record **before**
  invoking its factory to support cycles/partial exports; read the current
  `module.exports` on require rather than a stale `exports` object. Preserve
  the initial `exports === module.exports` alias and wrapper/`this` behavior.
  Cache states and error retry/caching policy require explicit compatibility
  tests; a cached promise is not a substitute for synchronous require.
- Build statically requested ESM names of CJS from reverse dependency
  occurrences, including renamed/re-exported requests; create synthetic proxy
  ESM segments dependent on the opaque CJS body/runtime. Default/namespace and
  named-property interop must have a documented target policy (Node-like versus
  esbuild/webpack conventions differ). `__esModule`, primitive exports,
  reassignment, getters, and post-evaluation mutation are acceptance cases.
  Unknown namespace access remains conservative, not a finite named list.
- Proxy helpers and user-proposed runtime snippets are design sketches, not
  proofs of live binding, cycle, or once-only semantics. A snapshot export of a
  CJS property does not become a live ESM binding automatically.
- Top-level await requires async evaluation scheduling across static dependency
  SCCs, shared evaluation promises, dynamic-import timing, and rejection
  propagation. P8.2 delegates those contracts to native Node ESM jobs, retaining
  TLA syntax. Require-of-async-ESM still gets source diagnostics; never silently
  lower to synchronous code.
- Maintain a tested support matrix: nonliteral import/require, direct eval,
  `with`, mixed ESM/CJS, import attributes, `import.meta`, top-level return,
  TS/JSX transforms, native addons/assets, externals, and require-of-ESM.
  Each construct is explicitly supported, externalized under a declared policy,
  or rejected with a span-bearing diagnostic; it must never disappear.

## 3. Phases and release gates

Each numbered checklist item in the companion TODO is intended as one narrow
commit/review. Dependencies are explicit there. This increment delivers P0;
subsequent increments follow P1–P9.

### P0 — Portable semantic fixture baseline

Import small licensed cases to `fixtures/bundler/esbuild/` and
`fixtures/bundler/rspack/`, each with upstream LICENSE and manifest provenance.
Use `NOTES.incremental-bundler.md`'s manifest exactly: full upstream commit,
original path/test, every adaptation, `moduleType`, ordered `entryPoints`,
deterministic stdout, and semantic goals. Never assume all upstream suites are
portable. No clone, bundler installation, downloads at test time, or unlicensed
copying.

The integration-owned driver validates manifests/paths, copies only case inputs
into a controlled workspace, supplies root package metadata only if absent,
runs entrypoints in separate Node processes in manifest order, and compares
their concatenated output lines. Reject traversal/absolute paths and missing
entrypoints; enforce timeouts and fail clearly on nonzero exit/stderr policy.
Test the driver itself with intentional stdout mismatch, malformed manifests,
timeout, and missing inputs. Existing fixture package metadata must win.

The delivered driver is `scripts/bundler-fixtures.mjs`; its synthetic regression
suite is `scripts/bundler-fixtures.test.mjs`, documented in
`fixtures/bundler/README.md`. Commands are `node scripts/bundler-fixtures.mjs`
and `node --test scripts/bundler-fixtures.test.mjs`. Configure their temporary
directory through the supported OS environment if needed; the runner uses
managed temporary directories and cleans them up. A project-local root is not
an architectural requirement. Integration ran both commands successfully
without a temporary-root override.

Seek deterministic observations of evaluation order, repeated imports,
dynamic-import identity, ESM cycles/live bindings, CJS cycles/reassignment and
interop where portable. Record absent coverage; do not invent claimed cases.
Native Node execution is a **fixture sanity check only**, not bundler conformance,
tree-shaking proof, chunk-count evidence, or proof of cross-entry shared state
(entries are separate processes). Later bundled mode needs a multi-entry
same-process driver for that property.

### P1 — Compatibility gates and source-local IR

Reproduce the known type-only regression in existing
`crates/unused_finder/src/test.rs`; add focused fixtures for raw/resolved report
paths, spans, aliases, forward lexical references, var hoisting, and exports
without runtime values. Correct tightly coupled bugs without mass migration or
changing expected behavior to hide them.

Create explicit parser/classification and diagnostic contracts, then owned
occurrence/binding DTOs with an unchanged unused-finder adapter. Validate after
arena reset. Add CJS opaque-body extraction and explicit unsupported diagnostics.
Document affected crates' READMEs before package changes (the inspected crate
roots currently have no README). Do not repurpose usage tags as effect flags.

### P2 — Parse cache

P2.1 implements `segment_cache::{CacheRecord, CachedModule, ParserConfig}`.
The deterministic CBOR envelope protects a versioned payload with SHA-256;
preflight limits allocations before serde and validates every source-local
range/link/order against the exact source. Existing owned facts/body are reused;
legacy maps/tags are not serialized. P2.2 adds `load_or_parse`: bounded sidecar
reads, explicit miss/write outcomes, optional read-only/disabled cache modes,
unique sibling staging and atomic replacement. Source rechecks prevent
detected edit/deletion races from publishing or returning stale success;
uncooperative editors are not part of a cross-file transaction. Independent
publishers are last-writer-wins with hash/config validation on every read.
Cache failures are nonfatal, but source read/parse failures remain fatal.
Pinned tempfile uses Unix rename / Windows MoveFileExW replacement; only Linux
was executed here. Cache orphans are ignored, never automatically swept.
P2.3 verifies exact cold/warm/disabled parse-fact parity, unchanged-mtime edits,
effective parser/package modes, relocation with the real resolver, source and
dependency deletion/orphans, current-path regenerated diagnostics and separation
from resolver/effects configuration. The public API is documented in the
[cache README](crates/segment_cache/README.md). P2 is complete; P3 is recorded below.

Implement deterministic DTO encoding/version checks separately from filesystem
IO. Add atomic sidecars and cache hit/miss reasons. Cover cold/warm equivalence,
same-mtime edits, parser changes, corruption, races, relocation, and write
failure. The cache must remain useful to unused analysis without requiring a
bundler or resolver and without changing autogenerated-file compatibility policy.

### P3 — Shared typed graph and resolution

P3.3 delivers explicit condition/external/symlink policy, resolver cache epochs,
graph source/resolution revisions, checked patch/add and tombstone deletion,
full occurrence refresh including misses and normalized rebuild parity.
Invalidation is intentionally whole-graph/whole-resolver initially; minimal
affected-set optimization remains P9.

P3.1 delivered `OwnedSourceFileInput` and `SourceGraph::from_owned` over the
existing parse/cache facts, plus typed occurrence edges and shared forward/
reverse indexes. Tests pin duplicate sites, empty identity and deferred require
metadata. Subsequent P3 items wire lexical/export linking, revision-aware
resolution and legacy usage consumers; P3.1 alone does not migrate TagGraph.

P3.2 delivers checked lexical/export resolution and canonical export identities
with barrel routes, plus typed binding-edge linking and diagnostics. The legacy
adapters retain their type-inclusive/empty-on-error API while correcting forward
lexical binding; executable consumers use the explicit namespace/error API.

Create canonical dependency occurrence edges and reverse index in
`source_graph`, with resolver inputs kept outside parsing. Add export resolution
correctness and generation-aware patch/add/remove. Resolve conditions for
`import` versus `require` separately. Keep a full-rebuild correctness oracle.
Adapt `tag_graph` only behind parity tests; converge
`report::compute_unused_segments` onto the verified graph in its own commit.

P3.4 completes that convergence. Both usage directions read the authoritative
forward/reverse indexes, including checked intermediate barrel routes; reports
share the same immutable graph through `ResultGraph::source_graph`. Legacy
tracker maps project once into shared edges, deliberately retaining their
type-inclusive lookup policy without claiming absent occurrence facts.
`OwnedSourceFileInput::from_ranges` links decoded cache facts without legacy
maps. Only the conservative report predecessor chain remains report-local.
Regressions preserve entry/test/ignore/type behavior, namespace/async/require/
side-effect traversal, empty identity and stale-tag invalidation; managed
filesystem tests fix static named-import barrels on file/symbol/segment surfaces
and prove reports no longer use the result's resolution maps. P4 must consume
structural evaluation/binding edges independently of `UsedTag`.

### P4 — Effects and retention

Implement conservative syntax effects and module-evaluation roots separately
from symbol roots. Retain effect argument evaluation, execution order, and
dependencies of retained effects. Add PURE annotation support only after base
tests; then add the effectful pattern DSL/config fingerprint. Distinguish
parser-derived facts from linker-derived conclusions in all snapshots.

P4.1 implemented in `ast_segmenter::effects`: ordered owned reason sites with
explicit deferred function/default-argument/instance-field timing. Cache
schema/extractor 2 persists syntax facts only and validates their ranges.
Reason snapshots plus all 75 parser/cache tests and `scripts/spot-check` pass.

P4.2 adds library-only `bundler::retain` over owned `SourceGraph` inputs. Module
evaluation, binding demand, source effect order and original occurrence/timing
edges are separate plan data. Async boundaries are recorded, not crossed.
Checked barrels/cache parity and evaluation/type-only/cycle/deferred regressions
pass (8 bundler + 31 source_graph tests); spot-check and targeted clippy pass.

P4.3 persists syntax-only invocation/attachment/input facts (cache versions 3).
`bundler::effects::summarize` and `RetentionPlan::effects` derive reason sites and
explicit invocation-only recipes with original callee/reference, argument order,
spread consumption and optional-chain guards. Value demand restores used calls;
no source emission is claimed. All 122 parser/cache/graph/bundler tests, targeted
clippy and spot-check pass, including deterministic reason/recipe snapshots.

P4.4 adds bounded `bundler::policy::EffectPolicy`: validated canonical structural
invocation rules, global-vs-wildcard shadow/alias/computed/optional semantics,
add-only known-effect reasons with precedence over PURE, and SHA-256 effective
policy fingerprints. `retain_with_policy` requires no source/parser; cache-built
reanalysis works with source absent and preserves sidecar bytes/mtime/warm hits.
`RetentionPlan::is_current(graph, policy)` checks both identities. Explicit
binding-to-module evaluation edges make binding-root traversal complete for P5.
Normalized optional-member shapes and concise-arrow result contexts require
extractor 4 (schema remains 3); no caller policy enters source-cache identity.
All 252 tests across the eight scoped crates, targeted clippy and spot-check
pass. At that P4 checkpoint, P5 root sets/partitioning and P6 emission remained
unimplemented; P5 evidence follows.

### P5 — Exact root-set partitioning

P5.1 adds `bundler::chunks` stable entry/site identities and an independent
synchronous flood-fill reference oracle over existing retention constraints.
Canonical path-based node sets include empty module identities; namespace
binding demand stays separate from evaluation. Chain/diamond/SCC, repeated
sites, sync+async targets, dead sites, invalid roots and stale policy/revision
regressions pass (28 bundler tests); deterministic snapshots and spot-check pass.

P5.2 adds `chunks::partition` reachable-site fixed point and exact canonical
root-set chunk identities. Full node→set/set→chunk snapshots include dead
segments/empty identities. 48 deterministic random small graphs match an
independent module oracle and randomized module/entry/edge ordering, with
pairwise equal-set iff same-chunk checks. Nested async cycles, namespace/binding
seeds, sync+async sharing and tombstone/stale-plan regressions pass (32 bundler
+ 31 source_graph tests), as do targeted clippy and spot-check.
`SourceGraph::module_ids` enumerates live empty modules after tombstones; a live
file count must not be treated as an ID upper bound.

P5.3 adds `manifest::diagnostic_manifest`: deterministic entry/async facades,
synchronous chunk dependencies and explicit synthetic facade/module-identity
records. Facade initial evaluation/binding demands and code load sets stay
separate; shared chunks cannot imply eager evaluation. Shared cycles, nested
async boundaries, empty modules, external sites, deferred CJS code availability,
namespace-vs-evaluation demand, insertion-order invariance and stale graph/policy
checks pass (37 bundler + 31 source_graph tests). Three reviewed manifest
snapshots, targeted clippy and spot-check pass. No source emitter, CJS runtime,
proxy generation or async evaluator is implemented; stop before P6.
Final compatibility validation passes all 264 tests across ast-segmenter,
bundler, import_resolver, oxc_utils_parse, segment_cache, source_graph, tag_graph
and unused_finder. Existing unrelated warnings remain; no bundler clippy warnings.

Introduce stable entry/boundary identities and a straightforward flood-fill
reference implementation first. Add reachable-boundary activation fixed point,
SCC tests, canonical set grouping, and a diagnostic partition manifest. Assert
exact sets, not merely expected chunk counts. A later bitset optimization must
match the simple oracle. No physical emission or merge heuristics needed here.

### P6 — ESM rewriting and evaluation

P6.1 chooses checked UTF-8 original-source edits and pinned-OXC JS
parser/semantic validation. Whole-AST printing would reconstruct owned syntax
and only partially preserve leading comments at the pinned revision.
`bundler::rewrite` proves private lexical renames (including shorthand), overlap
rejection and preamble preservation. No source maps or TS/JSX lowering.

P6.2 exposes `emit::single_module`: current retention and checked actual source
drive statement removal and composed PURE recipes. The output remains native
ESM, preserving scope/hoisting/TDZ/live exports. Node tests validate runtime
observations and measurable tree shaking. Guarded PURE callee chains with
nested optional calls/private fields are diagnosed, not incorrectly rewritten.

P6.3 exposes `link::esm` over the existing ChunkPlan and graph. Exact root-set
JSON payloads contain rewritten source parts, not copied unshaken inputs.
Node 22.15+ synchronous host hooks assemble the retained parts of each original
lexical module in source order and compile it as native ESM. This avoids unsafe
cross-chunk lexical splitting, preserves instantiation/TDZ/live-binding/cycle
semantics and never evaluates all modules sharing a payload. Entry launchers
default-export the real namespace; runtime `loadEntry` preserves canonical native
namespace identity. Separate published directories have separate identities.
The returned owned artifact map includes the logical manifest and exact member
mapping. Physical `module_payloads` separately records whole-lexical-module
assembly reads; these may exceed a root's diagnostic load set without moving,
duplicating, merging or hoisting any node. Demand-minimal/async fetching is not
claimed. Source dynamic imports/TLA/CJS/externals remain explicit P7/P8 gates;
browser output and source maps are unsupported.

Five portable ESM fixtures/eight executions match native baselines and manifest
stdout. Independent tests cover cross-root lexical mutation, cycle hoisting and
forward TDZ, namespace exotic behavior/identity, once-only concurrent loading,
bundle isolation, effect order, measurable removal and linked PURE recipes.
P6 does not claim the remaining CJS/TLA fixture corpus or P9 publication.

### P7 — CJS registry and ESM proxy interop

Implement opaque factories/cache semantics first, then reverse-demand proxy
generation under the selected interop policy. Verify same-runtime once-only
evaluation across entry/chunk paths, cycles, reassignment, thrown factories,
default/namespace/named forms, and mixed ESM↔CJS boundaries. Reject unsupported
mixed-file syntax rather than guessing its interpretation.

### P8 — Async loading and top-level await

Connect partition manifests to native ESM chunk loading, namespace facades, and
shared load/evaluation promises. Preserve import promise timing and error
behavior. Add TLA scheduling including dependency cycles, rejection, concurrent
imports, and require/async incompatibility. Loading and evaluating are separate;
one shared chunk cannot create extra module evaluations.

P8.1 uses native source `import()` of canonical virtual module URLs, not new
wrapper modules or an eager chunk evaluator. `Artifacts::async_facades` exposes
the per-site namespace routes already represented in the partition manifest.
CJS routes resolve to the existing bridge via a promise continuation. Successful
payload reads are cached once; load errors can retry, immutable cached text is
not hot-repaired, and native ESM owns compilation/evaluation failure semantics.
Generated Node evidence covers identity, timing, nested activation and failures;
P8.2 retains ESM await/for-await in native lexical modules. Owned source facts
record only suspension spans; schema 5/extractor 6 invalidates previous caches.
`Artifacts::async_modules` is the derived runtime-static dependency closure,
including SCCs, not source truth. Require sites targeting that closure fail with
both require and originating TLA source ranges, including deferred requires.
Node's module jobs own scheduling, concurrent shared evaluation and rejection;
there is no synchronous TLA approximation or second registry. Generated Node
tests demonstrate chain/diamond and SCC order, hoisting/TDZ, shared rejection,
concurrent identity and native exit 13 for awaited dynamic deadlocks. All twelve
portable cases/fifteen executions now match native and manifest observations.
P9 orchestration, invalidation keys and transactional publication are not added.

### P9 — Incremental end-to-end builds and conformance

P9.1 now exposes `bundler::build::BuildSession`. Independent layer keys and
actual extraction/resolution/link counters preserve source-local cache identity.
The initial implementation intentionally uses conservative JSON/file-name
inventory and full later-layer rebuilding after changes, while reusing unchanged
parse products and per-source resolutions. Canonical bounded roots reject
symlinks/out-of-root targets. Publication and public-pipeline differential/
benchmark gates follow in P9.3–P9.4. P9.2 publishes immutable generations behind
one atomically replaced ownership pointer with a nonblocking OS writer lock.
Explicit stale pruning requires old-reader quiescence so lazy runtimes retain
valid payloads across successful rebuilds; only listed unmodified files and
empty known directories can be removed.

Build layer-specific fingerprints and invalidation:

| Changed input | Minimum invalidation |
| --- | --- |
| Source bytes / parser policy / extractor schema | Parse DTO and all dependent derived products |
| Package exports/imports, tsconfig paths, resolution conditions, additions/deletions, symlink policy | Affected resolution and graph users, then effects/partition/output as needed |
| Effective module classification (e.g. package `type`) | Reclassify; reparse when parser mode changes |
| Effect DSL / annotations policy | Derived effects, retention, partition, output; not unrelated local parsing |
| Entrypoints / dynamic boundary reachability | Root sets, partition/facades/output; no unrelated reparse |
| Runtime/emitter/target configuration | Relevant transforms/runtime/output; parse only if effective parse settings change |

Use reverse dependencies and generations for removals as well as additions.
Full rebuild and incremental build must have identical logical output,
diagnostics, and runtime observations. Publish outputs atomically only after
success; remove stale artifacts without touching user files. Add differential
fixture execution (native baseline versus emitted output) and document where
interop intentionally differs from native Node. Benchmark cold/warm/no-op/local
edit/export edit/resolve-config edit using `cargo bench`, with counters for
reparses, resolutions, and re-emissions. Optimization is deferred until measured.

## 4. Validation and project conventions

- Existing unit tests live inline in `ast_segmenter`, `source_graph`, and
  `tag_graph`; unused-finder integration tests are in
  `crates/unused_finder/src/test.rs` and parser tests in `parse/`.
  Preserve `test_segment_one_used_one_unused_declaration`,
  `test_segment_side_effect_keeps_deps_alive`, `test_transitive_re_export`,
  `test_indirect_typeonly_export`, and `test_typeonly_files_are_typeonly`.
- Use `insta` for new general snapshots (normalized IR, typed edges, root sets,
  diagnostics, emitted code) and runtime assertions for semantics; pretty output
  snapshots alone are insufficient. Add dependencies only with their feature.
- Filesystem tests use managed tempdirs. Current `test_tmpdir::TmpDir::new`
  wraps `tempfile::tempdir()`; the Node runner also uses managed OS temporary
  directories. Temporary path location is not a bundler architecture constraint.
- New errors use `eyre`, not additional `thiserror`; do not opportunistically
  rewrite existing errors. Use `#[expect(...)]`, never `#[allow(...)]`.
- Suggested targeted gates as changes land:
  `cargo test -p unused_finder test_typeonly`,
  `cargo test -p unused_finder test_segment`,
  `cargo test -p ast-segmenter`,
  `cargo test -p source_graph -p tag_graph`,
  and the new library's selected tests. Run the smallest relevant selector.
- `scripts/spot-check` runs ast-segmenter check/tests and a **whole-workspace
  check**; it is not a bundler conformance suite. Run it before future commits
  under repository rules. Integration ran it successfully for P0; the planning
  agent did not rerun it during documentation finalization.
  `.github/workflows/unittest.yml` also runs formatting/clippy and release tests.
  `TODO.followups.md` records a separate toolchain-health failure; CI currently
  mentions nightly-2024-10-25 while `rust-toolchain.toml` says
  nightly-2026-05-01. Do not expand phase 0 into unrelated CI repairs.
- If Bazel shell tests invoking Cargo are introduced, include
  `@rules_rust//rust/toolchain:current_cargo_files` and prepend the toolchain
  runfiles `bin` directory to `PATH`.

## 5. Choices that must be resolved, not silently assumed

These decisions govern subsequent implementation phases; none blocks the
completed P0 fixture baseline.

1. **Runtime target and interop:** P6 selects Node 22.15+ native ESM hooks.
   CJS named exports, `__esModule`, error caching and require-of-ESM remain P7.
2. **Emitter:** P6 selects checked source edits and native per-module compilation.
   No source maps or TS/JSX lowering; a browser/portable target needs a separate
   implementation, not silently different lexical or partition semantics.
3. **Module identity:** symlinks/casing/package context and mixed ESM/CJS policy;
   stable logical keys versus in-memory graph generations.
4. **Persistence:** codec/hash/version ownership, atomic replacement portability,
   cache write failure reporting, and source-adjacent cache opt-out.
5. **Async boundary identity:** source-site identity is the initial correctness
   contract; preserving IDs across edits can wait. Do not merge sites by target.
6. **Graph convergence:** how to retire the report's second graph while restoring
   the known type-only behavior, without making usage semantics bundler-specific.
7. **Effect pattern grammar:** precisely which structural patterns are accepted;
   no claim of full JavaScript pattern syntax in the first DSL.
