# Incremental bundler execution checklist

This is the current ledger; historical completed migration TODOs are not
acceptance evidence. See `PLAN.incremental-bundler.md` for contracts and precise
current-code findings. Each ID is a narrow commit/review unit. `Depends` means
all listed prerequisites, not merely the preceding item.

**P0–P5 are complete. P6–P9 remain pending.** Both
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

- [ ] **P6.1 — Select and prove rewrite backend.** Depends: P1.4.
  Prototype pinned-OXC codegen or validated source edits; document source-map
  and TS/JSX policies. Tests: nonoverlapping edits, UTF-8, binding renames,
  shorthand properties, comments/directives/hashbang and parseable output.
- [ ] **P6.2 — Emit single-module ESM subset.** Depends: P6.1, P4.2.
  Preserve local evaluation and exports under retention. Runtime tests:
  lexical shadowing, var/function hoisting, forward lexical TDZ, live mutation,
  export aliases/default forms and effect order. Unsupported features fail.
- [ ] **P6.3 — Link multi-module and cross-chunk ESM.** Depends: P6.2, P5.3.
  Preserve instantiation versus evaluation, cycles and live exported bindings;
  resolve/emit namespaces and cross-chunk imports. Compare portable fixtures
  in bundled mode after P0.5. Acceptance: cycle/TDZ tests pass, not merely
  acyclic value snapshots; TLA remains explicitly rejected pending P8.

## P7 — CJS registry and interop

- [ ] **P7.1 — Implement opaque factory/cache runtime.** Depends: P1.5, P6.1.
  Cache module record before invocation; return current `module.exports`.
  Decide thrown-factory caching policy. Runtime tests: circular partial exports,
  exports alias, primitive/object reassignment, wrapper `this`, repeated
  require and failure/retry. Do not eagerly execute deferred require bodies.
- [ ] **P7.2 — Derive CJS proxy demands and policy.** Depends: P7.1, P3.2.
  Use reverse typed occurrences for static named/default/namespace requests;
  generate ESM proxy segments and runtime dependencies. Tests: renamed imports,
  re-exports, unknown namespace access, `__esModule`, getters and post-evaluation
  mutation/reassignment under an explicitly chosen interop contract.
- [ ] **P7.3 — Integrate proxies across chunks.** Depends: P7.2, P6.3.
  Feed synthetic segments through exact-set partitioning and share one registry
  per runtime. Same-process multi-entry tests verify once-only execution and
  mixed ESM/CJS cycles. Separate-process P0 tests alone do not satisfy this.

## P8 — Dynamic import and async evaluation

- [ ] **P8.1 — Load chunks with shared evaluation identity.** Depends: P6.3,
  P7.3. Implement dynamic facades, deduplicated loading/evaluation and namespace
  promises. Test repeated/concurrent imports, nested sites, sync+async reach,
  load failures, microtask timing and no extra execution from shared chunks.
- [ ] **P8.2 — Add TLA scheduling.** Depends: P8.1.
  Model async dependency/SCC evaluation and rejection propagation. Test async
  chains/cycles, concurrent requests, once-only evaluation and require of
  async ESM diagnostics. Never replace TLA with a synchronous approximation.

## P9 — Incrementality, conformance and measured performance

- [ ] **P9.1 — Connect layer fingerprints/invalidation.** Depends: P2.3,
  P3.3, P4.4, P8.2. Implement parse/resolution/effects/roots/runtime/output keys
  separately and reverse invalidation including deletion. Tests compare full
  versus incremental graph, root sets, diagnostics and emitted observations.
- [ ] **P9.2 — Publish transactional outputs.** Depends: P9.1.
  Stage only owned generated artifacts and publish after success; clean stale
  chunks safely. Tests: cancellation/failure, stale async chunks, source/cache
  watcher exclusions, no-op output stability and concurrent-build policy.
- [ ] **P9.3 — Complete differential fixture mode.** Depends: P0.5, P9.2.
  Run native baseline and emitted artifacts with explicit support/interop
  policies; add same-runtime multi-entry and bundler-only diagnostics cases.
  Report skips as unsupported, not passes; preserve original upstream
  provenance when adding adaptations.
- [ ] **P9.4 — Benchmark and release-gate.** Depends: P9.3, P3.4.
  `cargo bench` cold/warm/no-op/local/export/config edits with parse/resolution/
  emit counters; no criterion. Update library READMEs, concise adopted AGENTS
  contracts and support matrix. Run targeted tests and repository-required
  `./scripts/spot-check` before future commits. No heuristic chunk optimization
  until correctness is established and a separate plan authorizes it.
