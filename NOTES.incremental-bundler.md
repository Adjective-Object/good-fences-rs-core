# Incremental bundler implementation handoff

## Native-file replacement

The user revoked the previous hook/payload output architecture. It is removed
entirely. The replacement keeps the existing checked source edits, retention,
logical roots, cache/session and immutable publication, emitting one original
lexical module as a real `.mjs`/`.cjs` file. `.proxy.mjs` captures CJS named/default
requests; `.namespace.mjs` supplies runtime-key bridges. Ordinary relative file
imports/requires and native Node own identity, scope, TDZ, cycles and TLA.
Entries directly expose native exports, not a namespace-object launcher.

Logical exact chunks and physical files are deliberately distinct: all retained
statements in one lexical module share a physical file, even across root sets.
`members/chunk_artifacts` are overlapping projections, not exact physical
partitions. No segment-linker or demand-minimal physical fetching is claimed.
The previous all-var-contributions and guarded-PURE-await correctness fixes
remain covered by the existing public-pipeline regressions.

Native-file verification: `./scripts/bundler-release-check` passes 391 scoped
Rust tests (100 bundler), 43 harness regressions, all 12 native and 12 differential
contracts / 15 emitted executions, concurrent-entry static-export/identity and
four diagnostic cases, focused clippy and mandatory spot-check (51 parser tests
plus workspace check). Zero failures/ignored/unsupported cases. Old-reader
regression keeps a Node process running across rebuild/publication, then lazily
imports its original dependency before quiescent prune.
`cargo bench -p bundler --bench incremental` passes all six scenarios/seven
samples with native-file count/output-scan assertions: 66 files and 132 scans
per rebuild, zero writes/scans on no-op. Updated timings are in the bundler README.
Logs: `target/native-files-{release-check,bench}.log` (uncommitted local evidence).

## Historical P0–P9 handoff — superseded backend and measurements

Everything below is an archived checkpoint, not current architecture or current
acceptance evidence. All hook/code-payload/runtime-launcher statements and old
timings/counts describe the deleted backend only.

### Historical scope

P0–P8 are complete for the documented Node 22.15+ ESM/CJS target.
P0–P9 are complete for the documented Node 22.15+ support matrix.
P9 adds public build/session, transactional publication, public-pipeline
differential conformance, measured benchmarks and a repeatable release gate.

## P9.4 final verification

### Final-review binding checkpoint

- Local-reference edges and checked export routes now retain every declaration
  segment for the demanded lexical identity, including later `var` initializers.
  Same-segment declarations are deduplicated; unrelated variables remain dead.
- Reconstructed first-only behavior failed both the graph regression (one
  target instead of three) and real public BuildSession output (native `2`,
  generated `1`). Restoring the fix passes all 33 source-graph tests and the
  nine-case generated-output regression (direct/indirect exports, shadowing,
  no-initializer redeclarations and nested function/var merges).
  Mandatory `./scripts/spot-check` passed (51 parser tests + workspace check).
  Evidence: `target/review-binding-{graph,build}-{red,green}.log` and
  `target/review-binding-spot-check.log`. Full release gate follows the second
  final-review fix (now passed below); benchmark/support limits remain unchanged.

### Final-review await checkpoint and complete release gate

- Guarded PURE calls remain intact only if lowering would move source await
  into a generated arrow. Unguarded awaited receivers and existing async-function
  bodies still optimize; no generated async arrows or timing approximations.
  Deferred function bodies (including generator/yield bodies) remain outside
  PURE rewriting, so no yield-policy expansion was needed.
- Before the fix, both direct emitter and public BuildSession regressions failed
  generated validation: "`await` is only allowed within async functions and at
  the top levels of modules." Afterward, four focused tests pass, including
  14 real native/generated guard/effect/microtask cases (optional calls, members,
  computed keys, intermediate receivers and nullish branches), plus safe-erasure
  checks. Full bundler suite: 98 passed. Focused clippy and spot-check passed.
  Evidence: `target/review-await-{red,green,bundler,clippy,spot-check}.log`.
- The entire `./scripts/bundler-release-check` passed after both fixes:
  **389 scoped Rust tests**:
  bundler98, ast-segmenter51, good_fences60, import_resolver18, parse-utils9,
  segment_cache26, source_graph33, tag_graph22, unused_finder72. Also passed
  **43 harness regressions**, native12, differential12 / emitted15, same-runtime
  concurrent-entry identity, four source diagnostic cases, local all-target
  clippy `--no-deps -- -D warnings`, and mandatory spot-check (parser51 +
  workspace check). Zero failed/ignored/unsupported cases.
  Evidence: `target/review-final-release-check.log`.

### Prior P9.4 performance and release implementation

- Real `cargo bench -p bundler --bench incremental`, seven fresh managed roots
  per cold/warm/no-op/local/export/config scenario, 66 reached sources/65 requests.
  Median total ms: **127.282 / 72.650 / 4.824 / 73.442 / 76.044 / 75.966**.
  Source extractions: **66/0/0/1/1/1**; resolutions **65/65/0/1/0/65**;
  link calls **1/1/0/1/1/1**; generated-output/recipe parses
  **134/134/0/134/134/134**; artifact writes **3/0/0/3/3/3**.
  Actual min/max/build-time portions and hardware/reproduction are in README.
- The initial benchmark exposed 66 redundant emitter source re-extractions.
  BuildSession now reuses its own hash-checked existing CacheRecord projection
  through the same linker; public low-level `link::esm` remains source-checking.
  Generated code is still reparsed/semantically checked, with real parser
  call-site counters. No inflated/hidden source parse counts. Expanded tests
  compare disabled-cache full rebuilds, exact graph/root sets/artifact maps,
  runtime observations and the original checked emitter after every edit.
- Additional gates reject excluded-directory sources, browser build targets,
  tsconfig extends/references and ambiguous/unsupported wildcard mappings rather
  than silently ignoring config or exposing compatibility-resolver panics.
  Successful products carry session-local monotonic generations; graph handles
  never cross snapshots and no-op retains product identity. Writer ownership
  now spans the whole `build_and_publish` operation, with busy rejection tested
  before parsing as well as overlapping publication. Offline explicit snapshot
  publication follows commit order, not source-event chronology.
  Final parallel testing exposed an inherited-file-descriptor lock lifetime race
  after close; explicit unlock on success/error/unwind fixes it. A deterministic
  duplicated-descriptor + unwind regression now covers that case. Unlock errors
  after commit are returned warnings, not misleading rollback-shaped failures.
- Linux7.2.8 x86_64, Ryzen7 7840U, Rust1.97.0-nightly
  (f53b654a8 2026-04-30), Cargo1.97.0-nightly, Node22.22.2.
  Logs: `target/p9-4-{release-check,bench-final,invalidation,clippy}.log`.
  Earlier measurement: `target/p9-4-bench.log`.
- True limits, not failing/ignored acceptance cases: coarse config inventory and
  later-layer rebuilds; Node native synchronous physical reads; deliberate CJS
  bridge semantics; no browser/TS/JSX/assets/externals/import.meta/maps; canonical
  bounded projects and direct strict-JSON tsconfig subset; Windows unexecuted;
  old-reader quiescence before bounded prune; atomic visibility, not power-loss
  durability. No full-workspace test suite or unrelated CI repair claim.

## P9.3

The `build` Rust example is a thin invocation of BuildSession/build_and_publish,
returning generation paths/counters as JSON. Separate Node differential mode
reuses baseline manifest validation, copies native/source trees independently,
builds all entries together, then compares all native/generated/manifest output.
All 12 cases/15 emitted entry executions pass with zero unsupported cases.
An authored same-process concurrent-entry CJS/ESM identity check and four
bundler-only source-range diagnostics pass. Unexpected build failures fail;
explicit capability skips report unsupported with reasons and zero executions.
Original baseline runner/fixtures/provenance/licenses remain untouched.
Runtime timeouts/output bounds/stderr/exit failures clean managed copies; the
runtime child is synchronously killed/reaped. Detached descendants are outside
the trusted fixture contract. CI adds an isolated Node22/repository-Rust job.
Verification: 43 harness regressions (31 unchanged baseline + 12 differential),
12 native contracts, 12 differential contracts/15 emitted executions + identity/
4 diagnostics, all 90 bundler tests and `./scripts/spot-check` pass on Node
v22.22.2. Logs: `target/p9-3-{build,harness,baseline,differential,tests,spot-check}.log`.

## P9.2

`BuildSession::build_and_publish` and `publish::publish` install immutable
content-identified generations and atomically replace one bounded ownership/
current pointer. Source/build errors and checked cancellation before commit
preserve previous bytes/mtimes. OS locks reject concurrent publishers.
`current_directory` resolves fresh runtime paths; returned Published paths are
generation-specific, not mutable public launcher aliases.
Old generations survive publishing because running runtimes can lazily import
later. Explicit `prune_stale` requires quiescent old readers, removes only named
hash-unmodified owned regular files and empty known parents, and reports errors/
warnings while preserving user files, modified artifacts and symlinks.
No recursive cleanup, crash-orphan sweeping or power-loss durability claim.
All reserved staging/output/metadata and source sidecars are watcher-excluded.
Verification: all 90 bundler tests, focused all-target clippy (`--no-deps --
-D warnings`) and mandatory `./scripts/spot-check` pass. Publication tests include
actual overlapping threads/OS lock contention, all three cancellation/error
checkpoints, stale async removal, symlink/modified/user-file protection and
no-op byte/mtime equality. Logs: `target/p9-2-{tests,clippy,spot-check}.log`.

## P9.1

Historical checkpoint: P9.4 above removes the duplicate emitter source checks
described here, retaining only real generated-output/recipe validation parsing.

`BuildSession::build` uses source cache → StructuralResolver → owned shared
SourceGraph → effects/partition → link::esm. Separate parse/resolution/effects/
roots/runtime/output keys; source extraction, cache hits, resolver calls,
linker calls and required emitter source-check parses have distinct counters.
No-op has no parsing/resolution/emission; policy/root edits reuse source facts
and occurrence resolutions. Config inventory hashes JSON/JSONC bytes plus file
names under a canonical bounded project root, excluding reserved generated/cache
paths. This intentionally over-invalidates resolution; later layers rebuild
coarsely. Reverse users come from the shared graph, including deletion.
Symlinks/out-of-root targets are explicitly rejected. Existing resolver syntax
limits apply (bare-string exports rejected); no new resolution implementation.
Failed builds do not return previous successes; source/config rechecks precede
successful results. Tests cover cold/warm/no-op/policy/root/local/export edits,
package exports/type/tsconfig/options, deletion/recreate/rename and failure repair,
comparing graph, exact sets, diagnostics and complete artifact maps to fresh builds.
Verification: 85 bundler tests pass (including native emitted observations after
local edits); `./scripts/spot-check` passes 51 parser tests and workspace check.
Logs: `target/p9-1-{tests,spot-check}.log`. No full-workspace test claim.

## P8.2 / P9 handoff

- `ModuleFacts::top_level_await` contains ordered source await/for-await spans
  outside functions. CJS stays synchronous. Cache schema **5** / extractor **6**
  persists/validates these facts; all old sidecars miss. No async dependency/SCC
  state or target policy is cached as source truth.
- `link::Artifacts::async_modules` derives source-TLA + static dependency closure
  through cycles. Type-only/dynamic edges and async function bodies do not mark
  importers async. Every direct/transitive/deferred require of async ESM fails
  with require and originating TLA paths/ranges.
- Native Node ESM jobs schedule suspension/dependency SCCs and cache execution
  failures. Tests exercise chains/diamonds, cycle order/hoisting/TDZ, shared
  rejection, concurrent entries/imports and once-only evaluation. Awaited dynamic
  deadlocks retain Node's unsettled-TLA exit 13; no timers/cycle breaking.
- All **12 portable fixtures / 15 generated-artifact entry executions** match
  native stdout and manifest contracts, with zero skipped async fixtures.
  Cached owned facts → SourceGraph → retention → exact chunks → linked runtime
  produces identical output to cold parsing. A dynamic CJS factory and its
  deferred target demonstrably share one payload without executing the target.
- P9 starts from `segment_cache::load_or_parse`, `OwnedSourceFileInput::from_ranges`,
  graph revision/resolution APIs, `chunks::partition`, and `link::esm`. Existing
  `Artifacts::{files,entries,runtime,modules,module_payloads,members,
  chunk_artifacts,manifest,cjs_proxies,cjs_namespaces}` now also expose
  `async_facades` (per-site namespace tokens) and `async_modules` (derived paths).
  Native runtime exports remain `loadEntry`, `requireModule`, `commonjsNamespace`.
  The Rust library stages only an owned artifact map: no public build/watch/CLI
  orchestration, publication transaction, reverse invalidation, performance
  baseline or content-addressed outputs have been implemented for P9.
- Limits: Node 22.15+ only; synchronous physical JSON reads (not demand-minimal
  async fetching), immutable successful payload cache, deliberate CJS namespace
  bridge/extra promise reaction, conservative deferred require rejection.
  Externals/assets, import.meta, TS/JSX lowering and source maps stay gated.
  Existing tracker behavior and tracker/CLI runtime requirements are unchanged.
- Final verification on Node **v22.22.2**, with managed test directories under
  `TMPDIR="$PWD/target/p1-test-tmp"`:
  - `cargo test -p bundler -p ast-segmenter -p import_resolver -p oxc_utils_parse
    -p segment_cache -p source_graph -p tag_graph -p unused_finder --quiet`:
    **311 passed**, zero failed/ignored (82 bundler, 51 parser, 18 resolver,
    9 parse-utils, 26 cache, 31 source-graph, 22 tag-graph, 72 tracker).
  - `node scripts/bundler-fixtures.mjs`: **12 native contracts passed**;
    separately, bundled-mode Rust tests execute all 15 emitted entry artifacts.
  - `cargo clippy -p bundler -p ast-segmenter -p segment_cache --all-targets
    --no-deps --quiet`: passed; only pre-existing parser/ftree warnings.
  - `./scripts/spot-check`: passed parser check, **51 parser tests**, full
    workspace check. No full workspace test-suite or performance claim.

## P8.1 dynamic loading

- `Artifacts::async_facades` maps reached occurrence IDs to canonical namespace
  tokens. Native source imports preserve ESM promises/namespace identity; CJS
  imports resolve the existing runtime-key bridge with one extra reaction.
- One runtime caches successful payload reads. Load/read failures retry; cached
  payload text is immutable. Compilation rejects and native execution failures
  remain cached. No new evaluation registry or per-site wrapper modules.
- Real Node v22.22.2 tests cover concurrent/distinct/nested/dead sites, sync+async
  sharing, CJS-source imports, microtasks, missing payload retry, bad JavaScript
  rejection and shared execution failures. Portable coverage is nine cases /
  twelve executions; three TLA cases remain for P8.2.
- Validation: scoped parser/bundler/resolver/cache/graph/tag/tracker tests:
  **299 passed**, zero failed/ignored (72 bundler); `./scripts/spot-check`
  passed ast-segmenter check/tests and the full workspace check.

## P7.3 runtime / P8–P9 handoff

- Existing `link::esm` now accepts mixed JavaScript ESM/CJS graphs. `Artifacts`
  retains all previous fields and adds `cjs_proxies: path -> (token, names)` and
  `cjs_namespaces: path -> token`.
  `modules[path]` is the raw canonical factory/module token. `cjs_proxies[path]`
  is the static ESM proxy token; `cjs_namespaces[path]` is a native module whose
  default is the canonical CJS namespace bridge. `requireModule(raw_token)`
  returns native current require results. `loadEntry` returns native ESM
  namespaces for ESM entries and the runtime-key bridge for CJS entries.
- `ModuleFacts::commonjs` is the authoritative effective parser format, including
  package-commonjs `.js`. `ExportIdentity::CommonJs(file,name)` is a dynamic
  value request, not guessed export analysis. Cache schema 4/extractor 5 validates
  this bit against the parser config/body; old sidecars safely miss.
- `NodeId::CommonJsProxy { module, name: None }` initializes the proxy/default;
  `Some(name)` owns a requested named snapshot. These live in ordinary exact
  root-set payloads and manifest synthetic/dependency records.
  `NodeId::CommonJsNamespace(path)` owns the runtime-key bridge. Namespace/root
  demand includes that conservative bridge; require alone never demands a proxy.
  Reverse typed requests resolve names through checked barrel identities.
  Unused named imports preserve evaluation without invoking their getters.
- One runtime's Node-native CJS cache supplies early records, partial cycles,
  aliases/wrapper this, current replacement values and eviction/retry on throw.
  Native ESM proxies supply immutable namespace identity; default captures the
  complete require value ignoring __esModule, requested names/getters snapshot
  once in sorted order. Unknown namespace demands enumerate the actual require
  result's own string keys once, then forward reads/getters live without eager
  getter execution. The frozen/null-prototype bridge is not an ESM exotic;
  static ESM proxies remain native. New keys use ns.default; replacement affects
  subsequent require, not captured values. Failed ESM proxy
  evaluation remains cached by native ESM.
- CJS entry facades require before importing the proxy, preserving CJS-first
  cycles. Require-of-synchronous-ESM conservatively retains its whole namespace.
  Static ESM routes use file-relative virtual identities: Node's synchronous
  ESM job rejects custom-protocol specifiers before invoking resolution hooks.
  Legal deferred mixed cycles pass; native ERR_REQUIRE_CYCLE_MODULE is preserved
  when requiring a still-evaluating ESM. Source TLA remains rejected, not hidden.
- Physical `module_payloads[path]` includes factory + proxy pieces for CJS.
  Module/proxy assembly may read globally retained extra pieces without changing
  logical facade load sets, merging lexical environments or evaluating unrelated
  modules. A canonical proxy's named getters can therefore execute when first
  loaded even when requested by a different entry, as documented static surface.
- Eight portable synchronous fixtures/eleven executions now match native and
  case.json stdout. Four dynamic/TLA fixtures remain unsupported for P8.
  Same-process concurrent entry tests compare native observations, require/ESM
  object and namespace identity, once-only cross-file cycles and deferred timing.
- P7.1 pushed e345d78; P7.2 pushed 8ee7bc1. Each passed focused runtime,
  graph/cache compatibility and mandatory spot-check before commit. P7.2 also
  passed tracker/tag compatibility and local clippy. Formatter-only workspace
  churn is removed in P7.3; no unrelated behavior is changed.

## Final P7 verification

- Node v22.22.2; real filesystem/runtime tests use managed `test_tmpdir::TmpDir`
  under `TMPDIR="$PWD/target/p1-test-tmp"`.
- `cargo test -p bundler -p ast-segmenter -p import_resolver -p oxc_utils_parse
  -p segment_cache -p source_graph -p tag_graph -p unused_finder --quiet`:
  **296 passed**, zero failed/ignored (69 bundler, 50 ast-segmenter, 18 resolver,
  9 parser, 25 cache, 31 source graph, 22 tag graph, 72 tracker).
- Eight portable sync fixtures/eleven entry executions match native stdout and
  preserved fixture contracts. Seven dedicated CJS groups cover wrappers,
  aliases/primitive/object reassignment, hashbang/directive/early return, thrown
  retry, partial/self/cross-file/mixed cycles, getters/default/__esModule, named
  snapshots, conservative dynamic namespaces, package format, unused requests,
  exact synthetic sets and same-process concurrent multi-entry identity.
- Local bundler all-target `clippy --no-deps -- -D warnings`, mandatory
  `./scripts/spot-check` (ast compile/all 50 tests/full workspace check), and
  `git diff --check` pass. Existing dependency lifetime warning remains.
  Logs: ignored `target/p7-{1,2,3}-*.log`.
- Four source-async/TLA fixtures are unsupported, not passes. No full workspace
  test-suite, browser target, TS/JSX lowering, maps or transactional publication
  claim. P8/P9 start from the APIs/policies above; no P8 work is implemented here.

## Historical P7.1–P7.2 checkpoints

P7.2: `Artifacts::cjs_proxies` records canonical native ESM proxy tokens/static
surfaces. Reverse typed requests resolve original names through checked barrel
routes; CommonJs export identity is dynamic value demand, never export guessing.
Named/getter snapshots, full-result default ignoring __esModule, replacement
and namespace.default dynamic access are tested. `ModuleFacts::commonjs` closes
the package-format handoff gap; schema 4/extractor 5 rejects mismatched bits.
P7.3 subsequently moves proxy pieces into exact-set payloads and replaces the
intermediate finite namespace surface with conservative runtime-key bridges.

P7.1: opaque CJS bodies are rewritten only at checked require specifiers and
source hints, then stored in their original exact-set segment payload. Runtime
`requireModule(token)` uses Node createRequire and native cache, including
eviction/retry after throws. Native ESM's CJS translator was experimentally
rejected: its private require bypasses registered resolution hooks. Static
ESM-to-CJS imports remain diagnosed until P7.2 proxies.

The remaining P6 sections below are historical checkpoints, not current gates.

## P6.3 linker / P7–P9 handoff

- `link::esm(graph, plan, policy, sources)` returns `Artifacts::{files,entries,
  runtime,modules,members,chunk_artifacts,module_payloads,manifest}`. Caller owns
  publication; no output filesystem writes occur in the Rust library.
- Every live P5 node stays in its exact root-set payload. Checked static
  import/re-export routes and P4 retention/PURE recipes produce actual rewritten
  source parts; dead code is removed. Structural preamble/trivia/evaluation-only
  imports live in the existing module-identity chunk.
- Node 22.15+ synchronous `registerHooks` compiles source-order assembled
  original lexical modules as **native ESM**. Arbitrary root-set splits do not
  split lexical environments or replace live bindings with snapshots. Instantiation,
  cycles, lexical TDZ, hoisting and namespace exotic behavior remain host-owned.
  Reading a payload never evaluates other modules sharing it.
- Physical assembly reads all globally retained pieces of a requested lexical
  module. `module_payloads` records these reads separately from logical
  `manifest.load_chunks`. This is not demand-minimal/async fetching; no node is
  moved/duplicated/hoisted to manufacture a smaller partition.
- Generated `runtime.mjs::loadEntry(name)` returns the native namespace (or an
  ordered array for multiple evaluation modules). Launchers default-export that
  namespace object, not named snapshots. Use namespace binding roots for a full
  public namespace; selected roots intentionally retain less.
- Canonical virtual URLs include the published directory. Repeated/concurrent
  loads share one module identity; separate bundles do not share mutable state.
- P7 must supply CJS factory/cache/proxy policies without bypassing native ESM
  identity. P8 must implement source dynamic boundaries/TLA and any optimized
  fetching policy. Both remain explicit source diagnostics. P9 owns transactional
  publication/content hashes/invalidation; output filenames currently identify
  logical chunks, not content. No browser target, source maps or TS/JSX lowering.
- Five portable ESM fixtures/eight entry executions match native observations
  and case.json stdout; seven CJS/async fixtures are outside the declared target.
  Independent tests include cross-chunk lexical mutation, adversarial cycle
  TDZ/hoisting, namespace identity/shape, once-only/concurrent loading, runtime
  isolation, source effects, dead-code removal and linked PURE behavior.
- P6.1 pushed `6620eb0`; P6.2 pushed `a343238`. Both passed their targeted
  tests and mandatory spot-check before committing.

## Final P6 validation

- Real runtime: Node `v22.22.2`; managed `test_tmpdir::TmpDir` directories under
  `TMPDIR="$PWD/target/p1-test-tmp"` (no OS `/tmp` output).
- `cargo test -p bundler -p ast-segmenter -p import_resolver -p oxc_utils_parse
  -p segment_cache -p source_graph -p tag_graph -p unused_finder --quiet`:
  **289 passed**, zero failed/ignored. Counts respectively 62, 50, 18, 9, 25,
  31, 22, 72. Includes five bundled portable cases/eight executions, measured
  removal, equal-identity diamonds, adversarial cycle TDZ and builtin-mutation
  resilience. Stale source mapping/debugger hints are removed as comment
  metadata, without stripping matching strings or ordinary comments.
- `cargo clippy -p bundler --all-targets --no-deps -- -D warnings`: passed.
  Existing dependency lifetime warning remains; no bundler lint warnings.
- `TMPDIR="$PWD/target/p1-test-tmp" ./scripts/spot-check`: passed
  ast-segmenter check, all 50 ast-segmenter tests and full workspace check.
- `git diff --check`: passed. Logs are ignored artifacts:
  `target/p6-3-{compatibility,clippy,spot-check}.log`.
- P6.1 separately passed three rewrite tests and spot-check; P6.2 separately
  passed all 46 then-current bundler tests, targeted lint and spot-check.
  No full workspace test-suite, CJS/async bundled parity or P7–P9 claim.

## P6.1 rewrite backend

Checked original-source edits with pinned-OXC JS parser/semantic validation
avoid whole-AST reconstruction and the printer's partial comment preservation.
`rewrite::rename_private` uses owned lexical IDs and AST shorthand context;
public bindings/capture are rejected. No source maps or TS/JSX lowering.
Tests cover overlapping/invalid/UTF-8 spans, binding and assignment shorthand,
lexical shadows, comments/directives/hashbang and invalid generated identifiers.

## P6.2 single-module emission

`emit::single_module` checks retention freshness, reparses actual JS ESM and
checks facts/ranges against the shared graph before projecting retained source.
Dead statements disappear; directives/hashbang and unrelated trivia remain.
PURE recipes compose recursively, including optional-chain completion guards
and spread iteration. Terminal optional calls work; nested optional calls or
private fields inside a guarded PURE callee fail explicitly.
Native ESM supplies lexical hoisting/TDZ/live exports, not hand-written cells.
Six Node-backed test groups cover shadows, var/function hoisting, forward TDZ,
live mutation/aliases/defaults, measurable removal, getter/argument/iterator
order, late demand, nested recipes and unsupported/stale inputs.
Dependency-wide clippy finds existing ast-segmenter lint failures; use
`cargo clippy -p bundler --all-targets --no-deps -- -D warnings` for local lint.

## Historical P5.3 checkpoint / P6–P8 integration

- `crates/bundler/src/chunks.rs`: call `partition(graph, entry_RootSeeds, policy)`
  after constructing the owned shared graph. `ChunkPlan` includes final P4
  retention/recipes, exact roots, closures, node sets and chunks. There is no
  parallel source graph or usage-tag dependency.
- `crates/bundler/src/manifest.rs`: call `diagnostic_manifest(graph, plan, policy)`
  for deterministic `DiagnosticManifest::{entries,async_facades,chunks,
  chunk_dependencies,synthetic_nodes}`. Facades are separate artifacts, module
  identities keep their existing chunk, and empty modules remain represented.
- `Facade::{demand,load_chunks}` separates initial requested evaluation/bindings
  from transitive code availability. Async sites preserve consumer/target
  identity; source occurrence/function timing stays in the shared graph. No
  async relationship becomes a synchronous chunk edge. `ChunkPlan::retention`
  preserves effect order and source occurrence indices, not a total schedule.
- Every module still needs a distinct runtime evaluation record even when its
  code shares a chunk. Deferred CJS require regression proves only initial entry
  identity is requested by the facade, not all available lazy module bodies.
  Namespace-only pure bindings do not leak into evaluation-only root load sets.
- Check partition/manifest `is_current(graph, policy)` on the same graph
  instance; entry changes require recomputation too. Identity offsets and
  graph-local indices are revision-local; chunk IDs are canonical root sets,
  not content filenames. External nodes remain explicit occurrence placeholders
  with no local evaluation demand or executable external policy.
- Verified 37 bundler + 31 source_graph tests, three reviewed deterministic
  manifest snapshots and order-invariance checks, targeted clippy and mandatory
  `./scripts/spot-check`. P6 emitter, P7 registry/proxies and P8 loader/TLA remain
  unimplemented; no emitted runtime fixture/conformance claim.
- Final compatibility gate: all 264 tests pass across ast-segmenter (50),
  bundler (37), import_resolver (18), oxc_utils_parse (9), segment_cache (25),
  source_graph (31), tag_graph (22), unused_finder (72). Existing unrelated
  warnings remain; targeted bundler clippy has no new warnings.

## P5.2 checkpoint

- `bundler::chunks::partition(graph, entry_seeds, policy)` reuses P4 retention,
  activating individual reached async sites to a fixed point. Parent roots never
  cross async edges; namespace targets seed binding demand; externals stay explicit.
- `ChunkPlan::{retention,roots,reachability,node_roots,chunks,chunk_for,is_current}`
  exposes exact sorted sets. `ChunkId` is the nonempty root set, not an ordinal,
  filename or heuristic grouping. Dead source nodes have empty sets; live empty
  modules participate. Entry changes require recomputation, as do graph/policy.
- Added `SourceGraph::module_ids` because `file_count` excludes tombstones and
  cannot bound live IDs. Its regression removes an earlier empty module and
  verifies later live identities still participate in partitioning.
- Verified 32 bundler + 31 source_graph tests, reviewed exact mapping snapshot,
  targeted clippy and `./scripts/spot-check`. 48 seeded random graph cases match
  a separate module-level BFS/fixed point, random module/entry/retention-edge
  ordering preserves exact partitions, and all live-node pairs satisfy set
  equality iff chunk equality. This is diagnostic partitioning, not emission.

## P5.1 checkpoint

- `bundler::chunks::{RootId, AsyncSiteId, RootSeed, NodeId}` uses caller entry
  names and individual module-path/source-offset occurrences, not target paths.
  `async_seed` consumes a revision-local shared edge index.
- `reference_reachability` is the deliberately simple independent per-root
  synchronous flood fill over P4 retention edges; async/type-only edges are
  excluded. Namespace binding demand expands retained target statements without
  conflating ordinary module evaluation. Empty modules keep explicit identities.
- Verified 28 bundler tests, two reviewed deterministic oracle snapshots and
  `./scripts/spot-check`. Cases include chain/diamond/SCC, two sites to one target,
  sync+async overlap, dead sites and invalid/stale inputs. No emitted code.

## P4.4 checkpoint

- `bundler::policy::EffectPolicy` parses at most 128 rules of 256 bytes each:
  `name(...)`, `global.member(...)`, `*.member(...)`, ASCII identifiers and
  literal argument wildcard only. Patterns are structural, never JS execution,
  regex or source substring matches. Invalid grammar/limits return indexed eyre
  diagnostics, with deterministic snapshots.
- Exact global rules exclude lexical shadows/imports/parameters and do not
  infer aliases. Wildcard member receivers include lexical/aliased receivers,
  but do not follow standalone method aliases. Static/computed string literal
  names normalize; dynamic/template keys are not inferred. Optional members/
  calls and parenthesized member references preserve their original guards.
  Rules apply to both call and construction; nonmatches leave baseline policy.
- Matches add `KnownEffect` reasons and override PURE, never subtract syntax
  effects. Deferred function creation remains pure. Annotation disabling is
  explicit policy. Canonical sorting/deduplication plus a versioned SHA-256
  fingerprint identifies effective configuration independent of pattern order.
- `retain_with_policy` and `effects::summarize_with_policy` consume owned facts
  only. `RetentionPlan::policy_fingerprint` and `is_current(graph, policy)`
  invalidate derived results on policy-only changes. A managed filesystem test
  builds from a warm sidecar, removes source, reanalyzes two policies, restores
  source, and verifies another warm hit plus identical cache bytes/mtime/facts.
- Extractor 4 (schema 3) normalizes parenthesized optional-member shapes and
  preserves concise-arrow required return contexts. These are syntax changes,
  not caller-policy conclusions. Synthetic `BindingEvaluation` edges connect
  retained binding segments to their own module effects for later root walks.
- Verified 252 tests: ast-segmenter 50, bundler 25, oxc_utils_parse 9,
  segment_cache 25, source_graph 31, tag_graph 22, unused_finder 72,
  import_resolver 18. Targeted clippy and `./scripts/spot-check` pass; inspected
  reason, guard, policy-diagnostic and plan snapshots. Existing warnings remain.

## Retention/emitter handoff APIs

- Parse with `ast_segmenter::parse_module`, or get `CacheOutcome::record.module`
  from `segment_cache::load_or_parse`. Pass owned facts/ranges and current
  occurrence resolutions through `OwnedSourceFileInput::from_ranges` into
  `SourceGraph::from_owned`; do not reconstruct legacy tracker maps or UsedTag.
- Validate/resolve checked export routes with `SourceGraph::resolve_export`.
  Call `bundler::retain_with_policy(graph, evaluation_module_ids,
  binding_route_nodes, &EffectPolicy)`; `retain` uses the default policy.
- `RetentionPlan::{modules,segments,edges,async_boundaries}` describes available
  bodies, deterministic reasons and consumer-to-dependency structure.
  StaticEvaluation, BindingEvaluation, ModuleEffect and EffectOrder edges are
  derived constraints; source_edge indexes the original shared occurrence.
  Consult that occurrence's activation/function/site metadata. Async indices
  are retained boundaries; P5 maps them to separate `AsyncSiteId` roots. Do not eagerly execute
  deferred requires or interpret sorted plan edges as a total runtime schedule.
- `RetentionPlan::effects` maps SegmentKey to derived `StatementEffects`,
  including exact source-range `InvocationRewrite` inputs and `ArgumentGuard`.
  P6 must preserve callee/reference evaluation once, optional chain completion
  separately from undefined, argument order and spread iteration. Recipes are
  composed by source containment inside the original enclosing input guards,
  not applied as independent overlapping text edits. They are not an emitted
  program. Reuse only within the same graph instance and after
  `is_current(graph, policy)` succeeds.
- Genuine remaining limits: namespace demand keeps whole targets; required
  nested expression calls and segment-level binding dependencies are
  conservative. CJS stays opaque. The DSL intentionally has no arbitrary JS,
  dynamic-key inference or alias analysis. External edges are explicit but no
  executable external/runtime policy exists yet. P5 now supplies exact root
  sets, chunking and diagnostic manifests. Source rewriting, runtime wrappers
  and emitted conformance belong to P6 onward.

## P4.3 checkpoint

- `ast_segmenter::invocation` owns invocation/callee/reference/receiver/argument
  ranges, spread flags, original optional-chain boundaries, lexical callee IDs,
  syntactic `ValueUse` and exact attached PURE block-comment spans. Pinned OXC
  attachment plus exact comment-token matching rejects strings, prose, trailing,
  declaration-leading and separated false hints. Parentheses/trivia work.
- Cache schema/extractor 3 persists these syntax facts, never the decision to
  remove a call. It validates ranges, argument order, hint tokens, containment
  and binding links. Opaque CJS deliberately has no internal rewrite records.
- `bundler::effects::summarize` derives reason sites and `InvocationRewrite`.
  Recipes evaluate the original callee/reference once, then arguments in order
  with actual spread iteration and the original optional-chain/call guard.
  Chain completion state is distinct from a getter returning undefined.
- Only the unused invocation is discarded; used initializers/default exports,
  required argument/destructuring results and deferred bodies stay intact.
  Demand promotion restores calls and recomputes effect-order constraints.
  `RetentionPlan::effects` exposes all demanded-module statement summaries,
  including recipes for dead statements; P6 must consume these rather than
  treating an annotated whole statement as pure.
- Verified 50 ast-segmenter + 25 cache + 31 source_graph + 16 bundler tests,
  inspected optional/receiver/spread plan snapshots, targeted clippy and
  `./scripts/spot-check`. No emitter/runtime correctness claim.

## P4.2 checkpoint

- Created `crates/bundler/README.md` before the package. `retain(graph,
  evaluation_roots, binding_roots)` returns revision-stamped `RetentionPlan`,
  deterministic retained segment reasons, required module bodies, async boundary
  indices and additive consumer-to-dependency edges. Never consumes UsedTag.
- Runtime static imports/re-exports demand target evaluation independent of
  binding use; no-export/empty modules, transitive effects and cycles work.
  Type-only occurrences do not demand evaluation. Effect order edges and lexical
  dependencies survive; pure function/instance-field creation is not eager.
- Required deferred-require bodies remain available, with original structural
  occurrence/function metadata; this is not permission to hoist their execution.
  Namespace demand conservatively retains all target statements. Explicit
  external sites remain for P6 policy; no runtime/external support is claimed.
- `SourceGraph::{file_facts,outgoing_indexed}` exposes existing owned facts and
  revision-local shared edge indices. Checked export routes remain intact.
  Missing/legacy/invalid roots and link failures are errors, not empty successes.
- Verified: 8 bundler + 31 source_graph tests, cache-ranges/facts plan parity,
  inspected retention snapshots, targeted clippy and `./scripts/spot-check`.
  Existing dependency warnings are unchanged.

## P4.1 checkpoint

- `ast_segmenter::effects::{EffectReason, EffectSite}` supplies deterministic
  source-local reason sites through `ModuleFacts::effects`, independent of
  caller policy. None/deferred ranges separate creation from function/default
  argument/instance initialization. Opaque CJS always retains its whole body.
- Covers unknown calls/construction/control, getter/proxy reads, potential
  TDZ/unresolved reads, coercions, spread/iteration/destructuring, class heritage,
  computed keys/static blocks. This is conservative evidence, not execution
  scheduling or emitted runtime correctness.
- Cache schema/extractor 2 validates effect ordering, UTF-8 ranges and deferred
  containment; old sidecars miss. Cold/warm deterministic roundtrips still pass.
- Verified: 50 ast-segmenter + 25 segment_cache tests; reason/DTO snapshots
  inspected; `./scripts/spot-check` passes. Existing unrelated warnings remain.

## P3.4 checkpoint

- Legacy `SourceFileInput` projects lossy maps once in `source_graph/src/legacy.rs`
  into the authoritative edges. It intentionally lacks owned lexical occurrence
  facts; legacy type-inclusive lookup and absent/external-target tolerance stay
  compatible. Unlinked execution sites still retain their own usage tags.
- `TagGraph` reads `outgoing`/`incoming` in both directions, skips structural
  evaluation edges, follows checked barrel routes and conservatively expands
  namespace/execution module targets. Dynamic/require usage remains unchanged;
  this is not an eager-execution or bundler effects policy.
- Reports hold the same `Arc<SourceGraph>` in `ResultGraph::source_graph`;
  result-local file IDs must not be substituted for structural module IDs.
  Segment reports use TagGraph plus only the pre-existing non-hoisted predecessor
  chain. They no longer construct SegmentGraph or resolve result metadata.
  The standalone public ast_segmenter SegmentGraph remains for compatibility.
- Cache consumers can use `OwnedSourceFileInput::from_ranges` with decoded
  ranges/facts. The owned export index derives from facts, not legacy maps;
  cold/cache edges, export identity/routes and lexical resolution match.
- Regression evidence: five shared usage tests cover both directions, all
  usage root tags/type filtering, checked barrels, unrelated exports, namespace/
  async/require/execution, empty modules, external sites and revision reset.
  Real-filesystem barrel tests now retain both fully and partially used barrels
  on file/symbol/segment reports. Relative-dependency report tests clear result
  resolution maps to prove reports read shared structure.
- Validation (repository-local TMPDIR): `cargo test -p ast-segmenter
  -p oxc_utils_parse -p import_resolver -p segment_cache -p source_graph
  -p tag_graph -p unused_finder -p good_fences`: 284 tests
  (47/9/18/25/31/22/72/60), plus doc tests. `cargo clippy -p source_graph
  -p tag_graph -p unused_finder --all-targets` and `./scripts/spot-check`
  pass; existing unrelated warnings remain. No bundler conformance, execution
  scheduler or fine-grained P9 invalidation is claimed.

## P3.3 checkpoint

- Added explicit import/require `StructuralResolver` with exact external policy,
  per-mode caches and invalidation epochs. Existing node_modules caches key only
  by specifier: sharing them between conditions would return stale wrong-mode
  targets. Coarse invalidation replaces trees, including misses/config ancestry.
- Graph resolution refresh consumes every owned occurrence, preserving missing/
  external sites and returning diagnostics instead of old success. Package and
  tsconfig edits relink without changing source epochs. Source paths obey the
  resolver's explicit symlink policy; no hidden graph canonicalization.
- Patch APIs reject path-as-rename, remove tombstones IDs, recreation appends,
  both indexes/cache rebuild and versioned node handles expire on mutation.
- Verification: four new managed-filesystem graph tests cover missing→added,
  removed/recreated, mismatch rejection, separate conditions, package/tsconfig
  edits, external opt-in, symlinks and full-rebuild edge/diagnostic parity.
  30 graph + 18 resolver + 25 cache + 17 tag + 70 unused tests pass (160).
  `./scripts/spot-check` passes. No fine-grained P9 invalidation claim.

## P3.1 checkpoint

## P3.2 checkpoint

- Checked `resolve_export` handles explicit/default/aliased/imported-local and
  namespace exports, star default exclusion, same-identity diamonds, ambiguity,
  missing exports and cycle termination. `LinkedExport::route` preserves barrel
  segments, separately from canonical binding identity; owned binding edges use
  these routes. `validate_links` returns source-site construction diagnostics.
- `resolve_symbol_in_file` uses owned lexical reference IDs when present and
  fixes forward var/lexical binding in the legacy adapter. TDZ remains runtime
  behavior, not missing binding. The legacy import adapter intentionally returns
  type-inclusive leaves/empty on error; checked callers use explicit namespaces.
- Verification: 26 source_graph, 17 tag_graph and 70 unused_finder tests pass;
  spot-check passes. The report barrel regression is still owned by P3.4:
  old TagGraph does not yet consume the new routes.

- `source_graph::from_owned` consumes existing `ModuleFacts` and segments, with
  per-`DependencyId` resolution inputs, never a second parse IR. Typed edges
  distinguish binding/evaluation/execution, local references, activation,
  source site, read/write/type flags and enclosing-function metadata.
- Module nodes retain empty targets. Forward/reverse adjacency indexes the
  identical edge records. Duplicate specifiers and dynamic sites stay distinct;
  deferred CJS require stays synchronous with function containment recorded.
- Verification: 23 source_graph tests (two new) and `./scripts/spot-check`
  pass using repository-local TMPDIR. TagGraph/report migration is P3.4;
  missing/external resolution diagnostics and revision APIs are P3.3.

Phase 0 is historical planning/fixture work. The following P2 checkpoint records
its original bounded ownership; the current P3 completion is recorded above.

## P2.3 checkpoint — source-local independence and invalidation

- Cold, warm and cache-disabled records match `parse_module` facts, body and
  ordered statement ranges exactly, including ESM aliases/star/default exports,
  destructuring, lexical shadows/forward references, TS declarations/import
  types, Unicode, directives/hashbang and opaque CJS wrapper/deferred sites.
- Same-length edits with explicitly restored identical mtime invalidate by
  source hash. JS/TS/JSX/definition changes invalidate copied records. Effective
  parser modes match pinned parsing for all 11 supported extension forms and
  both package formats; package `type` changes reparse `.js`, while explicit
  ESM/CJS extensions override package policy and stay hits when appropriate.
- Relocated identical source/sidecars remain hits and re-resolve raw specifiers
  through real `MonorepoResolver` instances under distinct roots. Neither root
  appears in persisted bytes. Real resolver extension choices select different
  targets without changing source facts; package exports/sideEffects, tsconfig
  and external effects-policy edits leave cache bytes and mtime unchanged.
  No P4 policy evaluator or P3 graph behavior is claimed.
- Deleted source reads fail despite orphan caches; deleted dependencies fail
  fresh resolution while the importing source remains a hit. Identical source
  recreation may legitimately reuse its orphan cache. Diagnostics are never
  persisted: current source/path/policy regenerates errors, including generated
  markers, relocated ESM-as-CJS failure and non-UTF-8 source. Prior good sidecars
  survive errors but cannot provide stale successful results.
- Added a workspace OXC revision guard and kept decoder dependency-link checks
  linear using a source-local lookup map (not persisted). No resolver crate or
  legacy tracker behavior was modified. READMEs/TODO/PLAN document the public
  projection and the end of P2; P3 remains pending.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache
  -p ast-segmenter -p oxc_utils_parse -p import_resolver -p source_graph
  -p tag_graph -p unused_finder -p good_fences` passes **267 tests**:
  cache 25, parser/extractor 47+9, resolver 18, graph 21+17, consumers 70+60.
  Both existing owned-DTO insta snapshots pass unchanged.
  `cargo clippy -p segment_cache --all-targets --no-deps` passes with
  only the existing ftree_cache lifetime warning. `./scripts/spot-check` passes
  before every P2 milestone commit (47 parser tests and workspace compilation).
  Linux runtime validation only; Windows replacement behavior was inspected in
  pinned tempfile, not executed. Full-workspace tests/formatting are not claimed.

## P2.2 checkpoint — atomic source-adjacent sidecars

- `load_or_parse` reads exact source bytes, validates cache or parses, and
  returns source/record plus explicit hit/miss and write outcomes. Cache IO
  problems are nonfatal; source/parser/semantic/support errors are fatal.
  `CacheMode` provides read-write, read-only and disabled behavior. Negative
  diagnostics are not stored and old successes never bypass parse failures.
- Unique sibling staging uses existing locked tempfile 3.27.0. Its pinned
  implementation was inspected: overwrite uses Unix rename and Windows
  MoveFileExW with MOVEFILE_REPLACE_EXISTING, not delete-before-replace.
  Flush/sync precede publication; errors and unwinds clean staging files.
  Crash staging orphans are ignored; no writer sweeps another writer's files.
  This promises atomic visibility, not directory-fsync power-loss durability.
- Independent publishers are last-writer-wins. Re-read source immediately
  before publication and before returning both cold/warm results. Detected
  edits/deletion cannot publish/return stale snapshots; no cross-file lock
  transaction is claimed against noncooperating editors after the last check.
- Source files are untouched. Read-only sources work with writable parents;
  permission/replacement errors preserve previous records and return uncached
  facts plus an explicit write error. Requested lexical paths own sidecars;
  symlink source aliases have independent caches and symlink sidecar targets
  are replaced, not overwritten. Watcher artifact exclusion helper is public.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache`
  passes 15 tests. Added managed-directory coverage includes 64 concurrent
  publications with reads, interruption/error/unwind cleanup, write failures,
  source mutation/deletion races, stale parse success, cache modes, corruption/
  truncation/oversize repairs, permissions and symlink behavior.
  `./scripts/spot-check` passes (47 parser tests and full workspace check).
  Windows semantics are pinned-API inspected, not runtime-tested on this Linux host.

## P2.1 checkpoint — deterministic bounded codec

- New `segment_cache` crate reuses `ModuleFacts` and `ModuleBody`, with statement
  ranges as the source-local segment projection. It intentionally does not
  serialize legacy usage tags/name sets or create a parallel occurrence IR.
- CBOR schema/extractor versions and exact pinned OXC revision accompany source
  SHA-256/length and effective JS/TS/JSX/definition/CJS config. Parser defaults
  and checked extraction policy belong to the extractor version. A checksummed
  envelope, canonical re-encoding and byte/item/depth preflight reject damaged,
  ambiguous, oversized or incompatible data before trusting facts.
- Validate every UTF-8 range, IDs, binding/dependency links, occurrence ordering,
  and the one-body CJS invariant. Owned facts remain available after arena drop.
  serde_cbor 0.11.2 uses existing serde 1.0.218 without raising its constraint;
  SHA-256 0.10.9 was already locked. READMEs preceded source/package edits.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache`
  passes four tests, including five deterministic roundtrip modes, every-byte
  corruption/truncation checks and malformed length/depth/allocation inputs.
  `./scripts/spot-check` passes (47 ast-segmenter tests and workspace check).
  Only touched Rust files are rustfmt-formatted; workspace-wide fmt check
  exposes unrelated pre-existing formatting differences.

## P1.5 checkpoint — opaque CommonJS and explicit support gate

- `ParsedModule::body` / `ModuleBody::CommonJs { span }` identifies exactly
  one executable segment, spanning the complete body except hashbang, including
  directives, comments, whitespace and top-level returns. Even an empty CJS
  module has one opaque segment. No inferred CJS named export list or
  independently removable internal segments are returned.
- `Reference::commonjs` classifies unresolved require/module/exports wrapper
  occurrences independently of lexical shadows. `Dependency::enclosing_function`
  records lexical containment, not eager execution permission; after-return,
  repeated and nested dependencies remain separate. `DependencyKind::ImportType`
  retains TS type import sites without turning them into execution edges.
- Preserve `module.exports`, `exports` alias and module-variable reassignment
  verbatim. Reject require reassignment/aliases/indirect loaders, unsupported
  module member/escape accesses, wrapper-scope redeclarations and wrapper
  arguments escapes rather than guessing missing dependencies. Nested lexical
  and var shadows and type-space-only wrapper names have regression tests.
- `policy` rejects mixed ESM/CJS, nonliteral/unsupported loader shapes,
  unshadowed eval, nested with, import.meta, TLA/for-await, import attributes/
  options/phases, JSX and TS/decorator/auto-accessor/resource-management runtime
  transforms with path/range diagnostics. Erasable TS remains parse-only;
  ambient and merged type/value bindings and implicit type-only local/default
  exports retain accurate metadata.
- The ast-segmenter README support matrix explicitly leaves target-dependent
  asset/native-addon/external/require-of-ESM checks to later resolution/linking.
  Those raw specifiers are retained and tested, never silently omitted or
  claimed executable. Low-level parser and legacy adapters retain their broader
  policies. No P2 or later implementation was started.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p ast-segmenter
  -p oxc_utils_parse -p source_graph -p tag_graph -p unused_finder
  -p good_fences` passes 224 tests (47/9/21/17/70/60), including both insta
  snapshots and UTF-8 declaration/reference/dependency spans.
  `TMPDIR="$PWD/target/p1-test-tmp" ./scripts/spot-check` passes, including
  workspace compilation. Existing unrelated lifetime/dead-code warnings remain.

## P1.4 checkpoint — owned lexical and dependency occurrences

- Added `ast_segmenter::owned` DTOs via `ParsedModule::facts`, leaving
  `Segment`, `VariableScope`, `RawModuleDeps` and their legacy consumers intact.
  Binding IDs identify first declaration byte positions; dependency IDs
  identify expression/declaration byte positions within the exact source.
  Resolved lexical references do not depend on statement order. No OXC IDs
  or borrowed AST data escape.
- Preserve all binding declarations (including var redeclarations), references
  with read/write/type-only metadata, imported local aliases, local/default/
  string-named/star/namespace exports, repeated static/dynamic/require sites,
  directives and hashbang. Anonymous default expressions are not falsely
  identified as live local bindings. Nonliteral dependency calls fail.
- Tests cover lexical/var forward uses, nested closures, scope shadowing,
  object/array/default/rest destructuring, catch and method parameters,
  repeated imports/requires, type/runtime references and arena reset/reuse/
  drop. An insta snapshot pins deterministic owned DTO output.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p ast-segmenter
  -p source_graph -p tag_graph -p unused_finder -p good_fences
  -p oxc_utils_parse` passes 216 tests (39/21/17/70/60/9).
  `TMPDIR="$PWD/target/p1-test-tmp" ./scripts/spot-check` passes, including
  workspace compilation. Existing lifetime/dead-code warnings remain.
  CJS opaque extraction and the executable support matrix remain P1.5.

## P1.3 checkpoint — explicit parser policy and checked extraction

- `oxc_utils_parse::parse_bundler_file` uses extension-sensitive JS/JSX/TS/TSX
  syntax, an explicit package `ModuleFormat`, and explicit-extension overrides.
  Unsupported extensions and parser errors/aborts return eyre diagnostics with
  path and OXC diagnostic ranges. No package discovery or file IO is implied.
- `ast_segmenter::parse_module` validates semantic early errors before checked
  extraction and returns only owned `ParsedModule` records. A duplicate lexical
  declaration is accepted by the low-level parser but rejected at this gate.
  `try_segment_file` rejects unsupported statements and guards against partial
  statement output; legacy `segment_file` remains available for tracker callers.
- Generated markers do not skip bundler input, even when the source is invalid.
  Tracker compatibility still accepts TS in JS/unknown extensions, uses content
  module/script detection, and preserves its generated-file skip rule.
- Low-level parsing supports CommonJS syntax including top-level return.
  Bundler extraction explicitly rejects CommonJS pending P1.5's opaque body;
  TS import/export assignments and namespace exports also fail explicitly.
  Accepted TS/JSX records are not transformed/emitted JavaScript. Binding
  occurrences, directives/hashbang records and the broader support matrix
  remain P1.4/P1.5 work.
- Added eyre at workspace level for the new fallible APIs. Parser README existed
  before package edits; crate documentation records the separate policies.
- Verification: all 211 tests across oxc_utils_parse, ast-segmenter,
  source_graph, tag_graph, unused_finder and good_fences pass with managed
  temporary directories; `./scripts/spot-check` passes. Existing
  lifetime/dead-code warnings remain.

## P1.2 checkpoint — spans and resolved report dependencies

- Parser and public-report assertions pin zero-based, end-exclusive UTF-8 byte
  ranges, including offset zero, multibyte source, unused files/symbols/segments,
  and tagged type exports. Fixed the report test helper's stale `start - 1`
  slicing; production offsets are unchanged.
- Reproduced relative dependency reporting missing the first target segment:
  expected unused `[1]`, actual `[0, 1]`. Preserve the existing resolver mapping
  through `ResultFile` rather than looking raw specifiers up as absolute paths.
  Filesystem tests cover static/dynamic imports, re-exports, execution-only
  imports and require; clearing downstream symbol tags isolates segment edges
  from upstream tag propagation.
- Reproduced execution imports reaching no statements in no-export targets.
  Index each file's contiguous node range so execution-only imports and opaque
  requires reach all statements without scanning the whole graph per edge.
  Tests cover targets with and without exports. This is conservative usage
  analysis, not eager bundler evaluation of deferred require bodies.
- Verification: all 136 tests across ast-segmenter, source_graph, tag_graph and
  unused_finder and `./scripts/spot-check` pass. Filesystem tests use managed directories under
  `target/p1-test-tmp`. Existing lifetime/dead-code warnings remain.

## P1.1 checkpoint — type-only compatibility

- Reproduced with `cargo test -p unused_finder test_typeonly_interface_allowed`:
  `test test::test_typeonly_interface_allowed ... FAILED`, expected `[TypeOnly]`
  but actual `[Entry, TypeOnly]` on `MyInterface`.
- Root cause: extraction collapsed OXC type/value references into an untyped
  escaped-name set. Both tag traversals treated generic constraints as runtime
  intra-file dependencies. Retain a per-name type-only flag (AND across uses),
  and honor `follow_type_only` in forward and reverse traversal. No report
  filtering, changed expectations, or blanket suppression of type root tags.
- Added extraction coverage for constraints, annotations, implements, typeof,
  class inheritance, unresolved names, and mixed type/runtime references in
  both orders. Graph tests cover both traversal directions, both policies,
  entry/test/ignored tags, and explicit roots. Full-report filesystem tests
  cover ordinary/type imports and runtime re-exports; existing assertions stay.
- Verification: focused extraction and graph regressions pass; all four
  `cargo test -p unused_finder typeonly` tests pass. `cargo test -p ast-segmenter
  -p tag_graph -p unused_finder -p source_graph` passes 132 tests (27/17/67/21).
  `./scripts/spot-check` passes. Existing lifetime/dead-code warnings remain.
  Tests used managed directories under repository-local `target/p1-test-tmp`
  via `TMPDIR`, not system temporary paths.
- Adjacent probe, outside P1.1: a static import through a named re-export
  barrel reaches the leaf but leaves the barrel unused. With main importing
  `Consumer` from barrel and barrel re-exporting it from lib, the report diff was
  `unused_files: ["<root>/packages/root/barrel.js"]` and
  `unused_symbols: {"<root>/packages/root/barrel.js": [SymbolReport {
  id: "Consumer", start: 17, end: 50 }]}` instead of empty collections.
  Static-import resolution currently skips intermediate re-export segments.
  This separate linking/report issue is not normalized into a passing expected
  report or fixed here; the committed regression tests direct imports and
  re-exports. Revisit with P3 linking/shared-edge work.

## Completed phase-0 integration (historical)

- esbuild is checked out at `../esbuild`, pinned for this research to
  `f6058f8364fe7ab91ca57a83e02577ed74c9cae4`.
- Imported six esbuild and six rspack cases, with pinned provenance, adaptation
  notes, and upstream MIT notices including webpack's inherited notice.
- `node scripts/bundler-fixtures.mjs` passes all 12 baseline contracts
  (15 entrypoint executions); no bundler was invoked.
- `node --test scripts/bundler-fixtures.test.mjs` passes all 31 runner tests.
- `./scripts/spot-check` passes; this does not establish full-workspace test
  parity or resolve the pre-existing failures in `TODO.followups.md`.
- `.github/workflows/unittest.yml` runs the runner tests and corpus baseline
  in a separate Node 22 job without installing project dependencies.
- The scoped `.gitignore` exception preserves fixture `index.js` files.
  No fixture files remain ignored.
- `PLAN.incremental-bundler.md` and `TODO.incremental-bundler.md` describe the
  remaining increments. Source-local IR, disk caching, executable linking,
  effect analysis, chunk assignment, and bundle emission are not implemented
  by this increment.

## Phase-0 parallel ownership (historical)

- Architecture: `PLAN.incremental-bundler.md` and
  `TODO.incremental-bundler.md`; inspect current crates without modifying them.
- esbuild research/import: `fixtures/bundler/esbuild/` and
  `NOTES.incremental-bundler.esbuild.md`.
- rspack research/import: `fixtures/bundler/rspack/` and
  `NOTES.incremental-bundler.rspack.md`.
- Integration: fixture runner, shared documentation, validation, and commits.

## Phase 0 fixture interchange

Each upstream directory contains its upstream `LICENSE` and case directories.
Each case has `case.json` and a `files/` directory containing standalone JS
sources. Use this manifest shape:

```json
{
  "id": "upstream/descriptive-case",
  "upstream": {
    "repository": "https://github.com/owner/repo",
    "commit": "full commit SHA",
    "paths": ["path/to/original/test"],
    "test": "original test name",
    "license": "MIT",
    "adaptations": ["Describe every semantic or harness adaptation."]
  },
  "moduleType": "module",
  "entryPoints": ["entry.js"],
  "expectations": {
    "stdout": ["deterministic output line"]
  },
  "goals": ["Semantic properties exercised by the case."]
}
```

`moduleType` is `module` or `commonjs`. Entrypoints are relative to `files/`,
executed in manifest order in separate Node processes; expected stdout is the
concatenation of their output lines. Copy any required package metadata into
`files/`; the runner supplies a root package.json only when absent.
Cases must not require external packages, downloads, Jest, webpack/rspack
globals, an upstream bundler, or project sources. Replace upstream assertion
harnesses with deterministic runtime observations and document the adaptation.
Avoid adding output-format or chunk-count assertions: emitted-code shape is
not a portable semantic contract. Preserve license notices and record exact
provenance; do not copy fixtures with unclear licensing.

Baseline execution checks the fixtures, not bundler conformance. Linking,
chunking, diagnostics, and bundler-only cases will need explicit future test
drivers rather than being counted as passing by this runner.
