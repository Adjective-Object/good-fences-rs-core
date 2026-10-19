# Incremental bundler execution checklist

## Active segment-owned physical placement

- [ ] Replace input-module ownership/grouping with physical segment ownership.
  Prove one shared source splits across consumers and unrelated-source segments
  co-locate; do not substitute more module-merging heuristics.
- [ ] Preserve binding availability/initialization distinctions, TDZ, hoisting,
  imported writes, names, effect order and native namespace surfaces.
- [ ] Preserve native async suspension/deferred activation and opaque CJS
  segment/facade participation without an ESM runtime or per-source fallback.
- [x] Add `authored/native-segment-placement`: independent roots, shared effects,
  same-spelled inferred names, live state and read-only imports. Baseline corpus
  is 31 native-required cases / 43 executions; runtime equality alone does not
  prove segment relocation.
- [ ] Independently review the placement increment, run release/spot-check gates,
  then commit and push with exact supported scope and remaining gaps.

## Historical native-only cutover

- [x] Support native evaluation-only, multiple evaluation/binding owners,
  selected public/private binding-only roots, empty module identities and
  multiple public entry names for one source. Add actual Node observations for
  ordered effects, retained public surfaces, empty combined surfaces and identity.
- [x] Delete `physical.rs`, `physical_lower.rs`, `chunk_runtime.cjs`,
  Backend::Legacy, old module includes/fingerprints and unused symbol metadata.
  Both checked public and cache-reusing emission always use native output.
- [x] Require native for all 30 portable contracts / 41 executions.
- [x] Complete fresh `scripts/bundler-release-check`: 442 scoped Rust tests
  (151 bundler), 99 harness tests, 30 native-required cases / 41 executions,
  same-runtime identity + four diagnostics, strict clippy and mandatory spot-check.
  Fresh benchmark retains 66 reached sources / one file; README records measurements.
- [ ] Independently review the complete native-only deletion increment before
  committing. No commit, push, parity/oracle or upstream work in this increment.

## Historical replacement increments — preceding the final cutover

The ledger below records earlier increments and their then-current staging gates,
not active fallback requirements. Outstanding independent review is not inferred
from implementation or passing tests.

- [x] Unify the baseline/build corpus runner on base `ec0a859`: one
  `bundler-fixtures.mjs` command/API, input/build/backend/output failure results,
  continuation without building bad inputs, token-only status/legacy colors,
  and one corpus invocation in release/CI. Validated: 97 harness tests, public
  driver build, 23 contracts / 28 executions (10 native / 13 legacy / zero
  unsupported), and spot-check.
- [x] Independently review the unified runner increment; committed as `06f6c22`.
  No linker/backend gate changed in that increment.
- [x] Independently review the first harness diversion: authoritative production
  backend/layout reporting, native-required case metadata, optional fully checked
  upstream provenance, shared normalized runtime outcomes and preceding effects.
  Implemented and validated (89 harness/146 bundler tests, 14 baseline and
  differential contracts; four native, ten legacy), reviewed and committed
  as `6775e6c`.
- [x] Second harness increment: nine portable hoisting/names/destructuring/order/
  namespace/ambient-effect contracts; six additional native-required, three
  reported legacy. Four redundant Rust runtime tests retired (NOTES maps each).
  Validated: 142 bundler / 89 harness tests, 23 baseline + differential contracts,
  28 emitted executions; actual backends 10 native / 13 legacy; zero unsupported.
- [x] Independently review the second harness diversion; committed as `ec0a859`.
  No linker/gate changes or native namespace/async/CJS claim in that increment.
- [ ] For each subsequent linker stage, graduate applicable legacy fixtures to
  required native and/or add meaningful regressions exercising its new behavior.
  Compare against the fixture README's explicit backend baseline.
- [ ] Symbol linker: effect-retention/order analysis delivered; checked canonical
  symbols, production import/export resolution reuse and capture-safe name
  allocation implemented and validated. Independent review still required;
  the gated single-entry native lexical foundation is implemented.
- [ ] Review the native foundation: synchronous acyclic ESM, one namespace
  entry/root/group, direct bindings and explicit public aliases in `native.mjs`.
  Public BuildSession/publication and the 64-star-chain benchmark use it.
- [x] Extend the native slice to default expression snapshots/live aliases,
  anonymous default names, named declaration hoisting/TDZ and checked namespace
  dot reads/reexport aliases; admit anonymous syntax with imported-read rewrites.
- [x] Preserve names and read-only writes at native body boundaries when a merge
  cannot preserve them; never rewrite imported writes into defining bindings.
- [x] Use actual native chunks for full namespace values/public namespace exports,
  computed reads, descriptors, identity and receiver-sensitive accesses.
- [x] Scope-hoist exclusive synchronous dependency chains/body-free connectors;
  use native imports/exports across real chunks and native SCC declarations.
  Project logical artifacts to actual owners without changing root analysis.
- [x] Fix reviewed filtered-barrel surface widening: redirect only equal export
  name/identity maps; require native public-surface/false-ambiguity fixture.
- [ ] Independently review synchronous native grouping; extend pure/shared
  connector relocation and name-inference-safe merges where worthwhile.
- [x] Extend native linking to literal async roots/TLA/for-await through actual
  target chunk paths; keep async static closures/SCCs as native jobs, preserving
  sibling interleaving and source wait bounds. Graduate five legacy contracts
  and add portable siblings/nested activation, cycle TDZ/rejection/dedup and
  exported-then/default/live observations.
- [ ] Independently review the native async increment before committing.
  Selected-binding/multi-evaluation low-level roots remain staged.
- [x] Integrate one opaque CJS segment and logical ESM facade per input into
  normal usage/link/grouping, not standalone native `.cjs` fallback files.
  Pure availability groups use lexical cached initializers; facade jobs preserve
  demanded sorted snapshots, canonical bridges, deferred timing and mixed cycles.
  Graduate all five CJS cases and add activation/retry/cycle contracts:
  30 cases / 41 executions, all native (27 required), zero legacy/unsupported.
- [ ] Independently review native CJS integration before committing. Validate
  special low-level root cutover, then delete the old backend; no parity diversion.
- [ ] Remove legacy registrations/cells/aliases/watchers/generator dispatch,
  capture cycles and ESM per-source facade families, then delete the native
  staging gate. It is not a permanent fallback or public feature flag.
- [ ] Release after independent reviews, runtime/differential/invalidation gates
  and mandatory spot-check. Do not reuse historical completion claims below.

## Historical rejected architecture: physical exact-set emission (deleted)

- [x] Move real/synthetic executable registrations into their one exact chunk.
- [x] Separate binding storage/instantiation from original module execution;
  retain native dependency/TLA jobs and shared opaque CJS factory identity.
- [x] Add statement-sentinel, exact-file equivalence, import-closure, namespace
  TDZ/destructuring and iterator/global-scope probes alongside existing tests.
- [x] Fix deferred namespace initial fetching with native root capture cycles;
  verify unused mixed imports, namespace reexports, separate public target
  entries, direct dynamic target imports and deferred mixed-module requires.
- [x] Preserve labeled for-await continue/close and executable public CJS
  artifact paths without evaluating co-located entry bodies.
- [x] Revalidate the entire release gate and benchmark, and review executable
  output on adversarial root combinations. No per-module body fallback remains.

## Rejected historical native-file replacement

- [x] Delete the revoked P6–P8 loader backend and source-payload APIs entirely.
- [x] Emit native per-original-module `.mjs`/`.cjs` files, relative dependencies,
  direct static ESM entry exports, explicit CJS proxy/namespace files.
- [x] Keep exact logical roots; report overlapping physical module files honestly
  instead of claiming exact physical per-segment chunks.
- [x] Migrate public artifacts, publication, driver, harness and runtime tests
  to ordinary Node loading. Retain selective caches and immutable generations.
- [x] Revalidate native/differential contracts, live exports/TDZ/cycles/TLA,
  CJS cache/retry/deferred behavior, prior var/PURE-await fixes, no-op and
  running old-generation lazy loads. Rerun release gates and benchmark.

## Historical P0–P9 ledger — not the current output contract

The checked items below record the prior implementation only. Its P6–P8
hook/payload architecture was revoked and replaced above; those old checks are
**not** evidence of current exact physical chunking or current runtime APIs.
Old benchmark/test counts are historical. Current contracts/evidence are in
the bundler README and the physical-linker section of NOTES.

This is the current ledger; historical completed migration TODOs are not
acceptance evidence. See `PLAN.incremental-bundler.md` for contracts and precise
current-code findings. Each ID is a narrow commit/review unit. `Depends` means
all listed prerequisites, not merely the preceding item.

**P0–P8 are complete for the documented Node 22.15+ ESM/CJS target, including
source dynamic imports and native TLA scheduling. P9.1–P9.4 are now complete
for the documented support matrix.** Both
completed upstream research notes are incorporated in PLAN. Integration verified
12 runtime contracts (six esbuild and six rspack; 15 entrypoint executions),
31 harness regressions, and `./scripts/spot-check`. All P0 tasks are complete.
These P0 results do not establish bundler parity or full
workspace test success.

## P0 — Planning and portable fixtures

- [x] **P0.1 — Ground architecture.** Depends: none. Read root/ast-segmenter
  instructions and current parser/graph/report/test infrastructure; create
  PLAN and this TODO with evidence, gaps, dependencies, and verification gates.
  Acceptance: no crate changes or claims that existing graph APIs prove runtime
  correctness; known type-only regression explicitly recorded.
- [x] **P0.2 — Import esbuild cases.** Depends: none. Owner: esbuild corpus agent.
  Add a small standalone corpus, LICENSE, exact commit/path/test provenance and
  adaptation notes under `fixtures/bundler/esbuild/`; record research in its
  NOTES file. Acceptance: manifests conform to shared interchange and sources
  require no upstream harness, bundler, downloads, or external packages.
  **Delivered by owner:** six cases at pinned upstream
  `f6058f8364fe7ab91ca57a83e02577ed74c9cae4`, with independent native validation
  reported and all six cases accepted by the integrated shared runner.
- [x] **P0.3 — Import rspack cases.** Depends: none. Owner: rspack corpus agent.
  Same acceptance as P0.2 under `fixtures/bundler/rspack/`; distinguish native
  semantic observations from webpack/rspack-specific harness assertions.
  **Verified:** six cases at `96dcd72db4d25a89c37d22d514285ab82fc46a3d`,
  preserved rspack/third-party/webpack notices and provenance; all six pass the
  shared runner. Scoped ignore exception includes every `index.js` source.
- [x] **P0.4 — Build baseline driver.** Depends: P0.1. Owner: integration.
  Validate manifest/relative path contract, honor fixture package metadata,
  execute entries separately in order with timeout, compare deterministic
  stdout, document stderr/nonzero-exit policy. Test malformed/missing inputs,
  traversal, mismatch, timeout, and process cleanup in managed temporary
  directories; no crate modifications needed.
  **Delivered by integration:** `scripts/bundler-fixtures.mjs` and
  `scripts/bundler-fixtures.test.mjs`; all 31 synthetic regressions pass.
  OS-managed temporary directories worked without an override.
- [x] **P0.5 — Verify imported baseline and document coverage.** Depends:
  P0.2, P0.3, P0.4. Run driver over both corpora, record counts/results and
  missing semantic categories. README distinguishes fixture sanity from
  bundler conformance and separate-process from same-runtime entry tests.
  Acceptance: all 12 cases/15 entrypoint executions match manifests; no
  output-shape/chunk-count claims. Spot-check passed, no ignored fixture files
  remain, and an isolated Node 22 CI job runs fixture checks. Root README links
  the plan, TODO and fixtures README. This completes P0, not P1–P9 or full
  workspace testing.

## P1 — Compatibility and owned parse records

- [x] **P1.1 — Restore type-only compatibility gate.** Depends: P0.1.
  Reproduce `unused_finder::test::test_typeonly_interface_allowed` and fix the
  Entry-tag pollution without changing its expected `[TypeOnly]` behavior.
  Check adjacent `test_indirect_typeonly_export`/`test_typeonly_files_are_typeonly`
  and import/re-export propagation. Acceptance: focused regression passes.
  **Verified:** preserve type/value reference metadata during extraction and
  filter intra-file edges in both tag traversal directions. Original assertions,
  mixed runtime references, entry roots, and import/re-export regressions pass;
  132 tests across affected/dependent packages and `./scripts/spot-check` pass.
- [x] **P1.2 — Pin span and report-path behavior.** Depends: P0.1.
  Add UTF-8/0-offset/end-exclusive parse assertions and existing public-report
  compatibility assertions; reproduce/fix raw relative specifier misses in
  `report::compute_unused_segments` using resolved mapping. Check no-export
  effect imports and require dependencies. Acceptance: unused segment tests
  exercise real relative files, not only synthetic absolute paths.
  **Verified:** parser and public-report ranges remain zero-based, end-exclusive
  UTF-8 byte offsets, including offset zero and multibyte text. Reporting retains
  the resolver mapping for all five dependency forms. Execution-only imports and
  requires reach every target statement, including no-export files. Regression
  tests reproduced both gaps before fixes; all 136 affected/dependent tests and
  `./scripts/spot-check` pass.
- [x] **P1.3 — Introduce explicit parse policy and diagnostics.** Depends: P0.1.
  Add bundler parser mode/module classification contract alongside
  `oxc_utils_parse::parse_file` compatibility adapter; preserve existing tracker
  behavior. Surface semantic diagnostics and rejected statements, never partial
  executable output. Tests: JS/TS/JSX/CJS modes, parser/semantic failures, marker
  comments, unsupported syntax. Create/update crate README before package edits.
  **Verified:** fallible `parse_bundler_file` takes explicit package format,
  honors explicit ESM/CJS extensions, and rejects parser diagnostics.
  `parse_module` validates semantic diagnostics and returns owned records or an
  error; checked extraction cannot silently drop rejected statements. Legacy
  tracker policy and generated-file skipping remain unchanged. CommonJS
  extraction fails explicitly pending P1.5; TS/JSX parsing is not transformation.
  All 211 tests across parser/extraction and immediate consumers and
  `./scripts/spot-check` pass.
- [x] **P1.4 — Extract owned binding/occurrence IR.** Depends: P1.3.
  Extend extraction around `variables_for_top_level_statements` and
  `RawModuleDeps` with lexical IDs, declaration/reference spans, local import
  aliases, export mappings and individual dynamic sites. Keep old adapters.
  Tests: forward lexical/var references, shadowing, nested functions,
  destructuring, repeated imports, string-named exports, arena reset; insta DTO
  snapshots with deterministic ordering. No OXC arena IDs escape.
  **Verified:** additive `ParsedModule::facts` owns source-local binding IDs,
  all declaration/reference spans, type/read/write flags, aliases and export
  mappings (including destructuring, defaults, string names and re-exports).
  Dependencies preserve individual static/dynamic/require sites. Tests cover
  forward lexical/var resolution, redeclarations, nested/shadowed bindings,
  repeated specifiers, directives/hashbang, exact UTF-8 slices and arena
  reset/reuse/drop. Deterministic insta DTO snapshot and all 216 parser/consumer
  tests plus `./scripts/spot-check` pass; legacy adapters remain unchanged.
- [x] **P1.5 — Add opaque CJS and support matrix.** Depends: P1.4.
  Represent one executable CJS body plus dependency occurrences; classify global
  versus shadowed require/module/exports. Test reassignment, nested require,
  directives and top-level return policy. Nonliteral paths, mixed modules and
  unsupported transforms get tested diagnostics, not silently absent edges.
  **Verified:** `ModuleBody::CommonJs` has exactly one opaque body segment,
  excluding hashbang and preserving directives, reassignment and returns.
  Per-reference wrapper classification distinguishes lexical/type-only shadows;
  repeated/deferred sites retain source spans and enclosing-function metadata.
  Unsupported loader aliases/reassignment/module access/wrapper redeclarations,
  mixed modules, nonliteral paths, eval/with/import.meta/TLA, attributes/phases,
  JSX and runtime TS/decorator/resource transforms return span-bearing errors.
  Source-local matrix explicitly separates parse support from deferred
  resolution/runtime/TS-erasure checks. Type import sites, ambient bindings and
  merged type/value export metadata have regression coverage. Both insta DTO
  snapshots, all 224 parser/consumer tests and `./scripts/spot-check` pass.

## P2 — Versioned `.seg.cbor` cache

- [x] **P2.1 — Define codec and versioned DTO.** Depends: P1.4, P1.5.
  Select compatible CBOR/hash dependencies; encode source hash/length, schema,
  extractor/OXC version and effective parser config. Test deterministic bytes,
  roundtrip after arena drop, invalid spans, unknown versions and bounded decode.
  **Verified:** `segment_cache` reuses owned occurrence facts/body with ordered
  segment ranges, not legacy tracker tags/maps. Deterministic serde_cbor 0.11.2
  and SHA-256 work with locked serde 1.0.218. Version/config/source/checksum,
  UTF-8 range, ID/link/order checks and pre-allocation CBOR byte/item/depth
  limits reject malformed records. Four codec tests include ESM/CJS/definition
  roundtrips after arena drop, every-byte corruption/truncation, invalid
  spans/links/versions and allocation bombs; `./scripts/spot-check` passes.
- [x] **P2.2 — Implement atomic sidecars.** Depends: P2.1.
  Write `<source>.seg.cbor` via sibling staging/atomic replacement, with defined
  concurrent-writer and read-only-source behavior. Tests: corruption,
  truncation, interrupted replacement, source-change race and stale prior
  success after parse failure. Use managed test directories.
  **Verified:** `load_or_parse` reports hit/miss/write outcomes and supports
  read-write/read-only/disabled modes. Bounded reads, sibling staging,
  flush/sync/atomic replacement, source rechecks and RAII cleanup preserve old
  records on failure. Last-writer-wins, lexical symlink paths, read-only source
  and unwritable-parent behavior, Windows replacement semantics and crash
  staging-orphan policy are documented. Fifteen cache tests cover concurrent
  publishers, interruption/unwind, corrupt/truncated/oversized caches, deletion/
  edit races, old success after parser/semantic/support failure and filesystem
  write failures. Managed directories and `./scripts/spot-check` pass.
- [x] **P2.3 — Prove cache independence and invalidation.** Depends: P2.2.
  Cold/warm parse products match; same-mtime source changes and parser options
  invalidate; relocation re-resolves without embedding absolute paths.
  Tests: package-type mode change, deletion/orphan cache, cached diagnostic
  policy, resolver/effect config changes not contaminating source-local DTOs.
  **Verified:** 25 cache tests prove exact cold/warm/disabled facts/body/range
  parity, same-length/same-mtime edits, all effective parser modes, package-type
  changes and explicit extension overrides, relocated cache reuse with real
  re-resolution, deleted-source/dependency orphan handling and regenerated
  current-path diagnostics (including generated markers/invalid UTF-8).
  Real resolver extension changes select different files while source cache
  bytes/mtime stay unchanged; package exports/sideEffects, tsconfig and external
  effects-policy edits do not enter source-local identity. The OXC pin has a
  workspace identity guard. Targeted cache/parser/resolver/consumer tests,
  cache clippy and `./scripts/spot-check` pass. No P3 graph work is included.

## P3 — Shared structural graph

- [x] **P3.1 — Add typed occurrence edges and reverse index.** Depends: P1.4,
  P1.5. Build authoritative consumer→dependency edges in `source_graph`,
  preserving sync/async, type-only, evaluation/reference kind and source site.
  Tests: duplicate specifier occurrences, forward/reverse equality, deferred
  require timing metadata, empty-module identity. No `UsedTag` coupling.
  **Verified:** `from_owned` accepts existing parse/cache facts and per-site
  resolutions. `edges`/`incoming`/`outgoing` share one indexed collection;
  local references and import/re-export requests retain sites and flags.
  Two focused regressions plus 21 existing graph tests and spot-check pass.
  Legacy usage traversal migration remains P3.4.
- [x] **P3.2 — Correct lexical/export linking.** Depends: P3.1.
  Replace binding-by-statement-order assumptions for executable linking;
  implement namespace re-export, default exclusion from stars, explicit
  precedence, ambiguity/missing-export diagnostics and cycle-safe lookup.
  Tests target `resolve_symbol_in_file`/`resolve_import_across_files` adapters
  plus new linker APIs: TDZ-bound references, diamonds, aliases and cycles.
  **Verified:** explicit namespace linker returns canonical identities/routes
  and source-site diagnostics. Owned binding IDs resolve forward/nested uses;
  legacy forward lookup now binds lexicals. Three new focused linker tests,
  all 26 graph/17 tag/70 unused tests and spot-check pass.
- [x] **P3.3 — Add resolution revision/add/remove contracts.** Depends: P3.1.
  Read import-resolver AGENTS before changing it; adapt resolver into shared
  graph inputs with import/require conditions and external policy. Add
  generation-aware patch/delete and reject mismatched path-as-rename.
  Tests: removed target, new previously missing file, package/tsconfig changes,
  rename, symlink policy and stale reverse indexes; compare full rebuild.
  **Verified:** explicit resolver mode/external policy, separate condition
  caches, source/resolution epochs, stale handles, tombstone deletion and
  mismatched patch rejection. Four managed-filesystem tests cover all listed
  edits and normalized full-rebuild parity. 160 tests across resolver/cache/
  graphs/unused_finder and spot-check pass. Invalidation is coarse by design.
- [x] **P3.4 — Converge unused analysis on shared edges.** Depends: P1.1,
  P1.2, P3.2, P3.3. Adapt `TagGraph` traversals and
  `report::compute_unused_segments`, retiring duplicated dependency logic only
  after parity tests. Acceptance: type-only/ignore/test/file/symbol/segment
  reports preserved or explicitly corrected by regression tests; bundler policy
  never changes existing usage defaults.
  **Verified:** both TagGraph directions and segment reports traverse the same
  authoritative edges; legacy maps project once with checked barrel routes.
  Cache facts/ranges link without tracker maps. Five shared traversal regressions
  cover entry/test/ignore tags, type filtering, namespaces, async/require/
  execution edges, empty modules and stale state. Two managed-filesystem barrel
  regressions fix file/symbol/segment reporting, including partial barrels.
  All 284 tests across parser/resolver/cache/graphs/unused/good_fences pass;
  touched-crate clippy and `./scripts/spot-check` pass (existing warnings only).
  Report-only predecessor policy remains separate from structural effects.

## P4 — Effects and retention

- [x] **P4.1 — Add conservative syntax effect summaries.** Depends: P1.4.
  Test pure function declaration creation versus effectful invocation, unknown
  expressions, getters/proxies, coercion, spread/iteration, computed class
  members/static blocks and unresolved reads. Snapshot reasons, not just bools.
- [x] **P4.2 — Separate evaluation and binding roots.** Depends: P3.2, P4.1.
  Retain top-level effects when any runtime import evaluates a module, including
  unreferenced exporting declarations and modules with no exports. Test order,
  effect dependencies, nested body timing, type-only non-evaluation and cycles.
- [x] **P4.3 — Add PURE annotation semantics.** Depends: P4.2.
  Recognize precise comment attachment; preserve callee/argument/receiver effects
  when dropping an unused pure invocation. Test used results, spread arguments,
  optional calls, false attachment and `f(sideEffect())`; no substring shortcut.
- [x] **P4.4 — Add known-effectful structural DSL.** Depends: P4.3.
  Specify/parse a bounded grammar including `*.appendChild(...)`, add-only
  effect semantics and precedence over PURE hints. Test computed/optional/
  shadowed/aliased forms per documented policy, malformed patterns and config
  fingerprints. Verify no source reparse for only a DSL-policy change.

## P5 — Exact chunk sets

- [x] **P5.1 — Establish root/boundary identities and oracle.** Depends: P3.1,
  P4.2. Add entry and per-occurrence async root IDs; reference flood-fill follows
  synchronous edges only. Tests: chain, diamond, SCC, two sites to one target,
  one target reached both sync and async, unreachable import sites.
- [x] **P5.2 — Activate nested boundaries and partition.** Depends: P5.1.
  Compute reachable-site fixed point and canonical exact root sets. Snapshot
  every segment→set and set→chunk mapping; randomized insertion order must not
  change partitions. Assert equal sets iff same logical chunk; no merge/hoist.
- [x] **P5.3 — Plan chunk/facade dependencies.** Depends: P5.2, P3.2.
  Emit a diagnostic manifest of entry facades, async facades, chunk edges and
  synthetic nodes, separate from execution scheduling. Test shared cycles,
  nested boundaries and chunks containing separately evaluated modules.
  Acceptance: 37 bundler + 31 source_graph tests, deterministic facade/chunk/
  synthetic-node snapshots, targeted clippy and mandatory spot-check pass.
  Final compatibility gate: all 264 tests across the eight scoped crates pass.
  P5 produces diagnostic plans only; no emitted-runtime bundle claim.

## P6 — Correct ESM output

- [x] **P6.1 — Select and prove rewrite backend.** Depends: P1.4.
  Prototype pinned-OXC codegen or validated source edits; document source-map
  and TS/JSX policies. Tests: nonoverlapping edits, UTF-8, binding renames,
  shorthand properties, comments/directives/hashbang and parseable output.
  Checked UTF-8 source edits plus pinned-OXC parser/semantic validation;
  private lexical renames expand shorthand, reject capture/public bindings.
  JS-only emission, no source maps; see bundler README.
- [x] **P6.2 — Emit single-module ESM subset.** Depends: P6.1, P4.2.
  Preserve local evaluation and exports under retention. Runtime tests:
  lexical shadowing, var/function hoisting, forward lexical TDZ, live mutation,
  export aliases/default forms and effect order. Unsupported features fail.
  `emit::single_module` consumes current retention and owned facts/ranges,
  tree-shakes statements and composes effect-preserving PURE recipes. Real Node
  tests exercise scopes/hoisting/TDZ/live exports and optional-chain/getter/spread
  order. Unsupported modes/dependencies/callee chains return source diagnostics.
- [x] **P6.3 — Link multi-module and cross-chunk ESM.** Depends: P6.2, P5.3.
  Preserve instantiation versus evaluation, cycles and live exported bindings;
  resolve/emit namespaces and cross-chunk imports. Compare portable fixtures
  in bundled mode after P0.5. Acceptance: cycle/TDZ tests pass, not merely
  acyclic value snapshots; TLA remains explicitly rejected pending P8.
  `link::esm` returns staged artifacts: exact-set rewritten-source payloads,
  native Node ESM host hooks, launchers and physical assembly dependencies.
  The host compiles each original lexical module after source-order assembly;
  chunks never become eager evaluators or collapsed lexical scopes.
  Five portable ESM cases/eight executions match native and manifest stdout.
  Independent cross-chunk cycles/TDZ/hoisting, live namespaces/mutation,
  once-only/concurrent loading and instance-isolation tests pass.
  Node 22.15+ only; source async/CJS/externals, browser output, source maps and
  TS/JSX lowering remain explicit gates. Physical module assembly is not
  demand-minimal chunk fetching; no exact logical root assignments change.

## P7 — CJS registry and interop

- [x] **P7.1 — Implement opaque factory/cache runtime.** Depends: P1.5, P6.1.
  Cache module record before invocation; return current `module.exports`.
  Decide thrown-factory caching policy. Runtime tests: circular partial exports,
  exports alias, primitive/object reassignment, wrapper `this`, repeated
  require and failure/retry. Do not eagerly execute deferred require bodies.
  Native createRequire/hook loading owns wrappers/cache; thrown factories evict
  and retry. Real Node differential test covers every listed runtime behavior.
- [x] **P7.2 — Derive CJS proxy demands and policy.** Depends: P7.1, P3.2.
  Use reverse typed occurrences for static named/default/namespace requests;
  generate ESM proxy segments and runtime dependencies. Tests: renamed imports,
  re-exports, unknown namespace access, `__esModule`, getters and post-evaluation
  mutation/reassignment under an explicitly chosen interop contract.
  Reverse typed edges and checked barrel identities derive original names.
  Native ESM proxies snapshot named properties/getters; default is the require
  result, regardless of __esModule. P7.3 completes conservative runtime-key
  namespace bridges (live reads, snapshot keys), not a finite static name list. Format facts are
  explicit; cache schema 4/extractor 5 validates their parser-mode identity.
- [x] **P7.3 — Integrate proxies across chunks.** Depends: P7.2, P6.3.
  Feed synthetic segments through exact-set partitioning and share one registry
  per runtime. Same-process multi-entry tests verify once-only execution and
  mixed ESM/CJS cycles. Separate-process P0 tests alone do not satisfy this.
  Proxy initialization/default, runtime-key namespace and named snapshots are synthetic
  nodes in exact-set reachability/chunks/manifests and rewritten JSON payloads.
  One native registry/proxy identity survives concurrent same-process entries,
  cross-file partial cycles, deferred requires and synchronous require-of-ESM.
  Illegal evaluating-ESM require cycles preserve native error codes.
  Eight portable sync fixtures/eleven executions match native stdout/contracts;
  four async fixtures remain deferred to P8. No logical root-set relaxation.

## P8 — Dynamic import and async evaluation

- [x] **P8.1 — Load chunks with shared evaluation identity.** Depends: P6.3,
  P7.3. Implement dynamic facades, deduplicated loading/evaluation and namespace
  promises. Test repeated/concurrent imports, nested sites, sync+async reach,
  load failures, microtask timing and no extra execution from shared chunks.
  Literal source imports now route through per-site logical namespace facades to
  the shared native loader; ESM namespaces and CJS bridges retain canonical identity.
  Generated-artifact tests cover concurrent/distinct/nested/dead sites, sync+async
  identity, CJS-source imports, microtask order, read/compile/execute failures,
  read retry and immutable cached payload text. Nine portable cases/twelve entry
  executions pass; three source-TLA fixtures await P8.2.
- [x] **P8.2 — Add TLA scheduling.** Depends: P8.1.
  Model async dependency/SCC evaluation and rejection propagation. Test async
  chains/cycles, concurrent requests, once-only evaluation and require of
  async ESM diagnostics. Never replace TLA with a synchronous approximation.
  ESM await/for-await spans survive owned/cache/graph inputs (schema 5/extractor 6).
  The linker derives static async closure through cycles and rejects direct,
  transitive and deferred requires of async ESM with original source sites.
  Native Node jobs provide SCC suspension/order and shared rejection without a
  competing scheduler. Generated-artifact tests verify chains/diamonds/cycles,
  hoisting/TDZ, once-only concurrent entries/imports, rejection identity, exit 13
  for unsettled TLA, and cached-facts/cold-output equivalence.
  All twelve portable cases/fifteen executions pass native + generated output +
  manifest stdout checks; no async fixture skips or ignored failures.

## P9 — Incrementality, conformance and measured performance

- [x] **P9.1 — Connect layer fingerprints/invalidation.** Depends: P2.3,
  P3.3, P4.4, P8.2. Implement parse/resolution/effects/roots/runtime/output keys
  separately and reverse invalidation including deletion. Tests compare full
  versus incremental graph, root sets, diagnostics and emitted observations.
  Public `BuildSession::build` integrates the existing pipeline; distinct layer
  identities/counters, no-op reuse, warm parse reuse, reverse-user invalidation,
  deletion/recreation/rename, package exports/type and tsconfig edits are tested.
  Later graph/retention/output rebuilding and JSON/file-name inventory are
  deliberately coarse; generated-output validation is separately counted
  (P9.4 removes redundant emitter source re-extraction).
- [x] **P9.2 — Publish transactional outputs.** Depends: P9.1.
  Stage only owned generated artifacts and publish after success; clean stale
  chunks safely. Tests: cancellation/failure, stale async chunks, source/cache
  watcher exclusions, no-op output stability and concurrent-build policy.
  Immutable content-identified generations + one atomic ownership pointer,
  nonblocking OS single-writer locks, cancellation/failure tests at every
  precommit phase and unchanged bytes/mtimes are implemented. Old generations
  remain valid for running lazy loaders; explicit quiescent `prune_stale` removes
  only known unmodified files/empty parents, preserving foreign/modified files.
- [x] **P9.3 — Complete differential fixture mode.** Depends: P0.5, P9.2.
  Run native baseline and emitted artifacts with explicit support/interop
  policies; add same-runtime multi-entry and bundler-only diagnostics cases.
  Report skips as unsupported, not passes; preserve original upstream
  provenance when adding adaptations.
  Separate Node harness invokes a minimal Rust public-library driver and the
  transactional publisher: all 12 cases/15 emitted executions, one additional
  same-runtime concurrent multi-entry case and four bundler-only diagnostics.
  Explicit unsupported results are not passes. Baseline behavior/provenance/
  licenses are unchanged; Node 22 + repository-toolchain CI is wired separately.
- [x] **P9.4 — Benchmark and release-gate.** Depends: P9.3, P3.4.
  `cargo bench` cold/warm/no-op/local/export/config edits with parse/resolution/
  emit counters; no criterion. Update library READMEs, concise adopted AGENTS
  contracts and support matrix. Run targeted tests and repository-required
  `./scripts/spot-check` before future commits. No heuristic chunk optimization
  until correctness is established and a separate plan authorizes it.
  Seven real `cargo bench` samples/scenario publish timings and extraction/
  resolution/link/output-parse/artifact-write counters in the library README.
  Redundant source re-extraction during session emission was removed using the
  existing checked cache facts; full disabled-cache and original checked-emitter
  parity is tested after edits. Final review fixes all-declaration binding demand
  and guarded PURE/await lexical boundaries, with native/generated regressions.
  `./scripts/bundler-release-check` passes 389
  scoped Rust tests, 43 harness regressions, 12 native + 12 differential cases/
  15 emitted executions, same-runtime identity, four diagnostics, focused clippy
  and mandatory spot-check. Limits are explicit: coarse later layers, bounded
  canonical roots/limited direct tsconfig forms, Node-only synchronous payload
  reads/intentional CJS bridge policy, untested Windows publication, quiescent
  cleanup and atomic visibility (not power-loss durability). No full-workspace
  test suite, browser/transform/asset/externals or heuristic optimization claim.
