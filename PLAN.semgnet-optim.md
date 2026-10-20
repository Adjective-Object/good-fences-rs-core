# Segment optimization implementation plan

Implement these three features in order. Reuse existing reachability masks,
placement constraints, and native emission. Do not introduce a separate demand
model, coverage collection, or probability-based selection.

Native ESM remains the default execution model. Ordinary modules and chunks keep
native imports, exports, bindings, and evaluation with no runtime wrapping.
Introduce a narrowly scoped initialization helper only when a specific
transformation requires deferred activation that native placement alone cannot
preserve. Emit helpers only for affected islands; never route unrelated modules
through them or introduce a global module registry.

## 0. Parent-hoisting across async boundaries

Goal: a child used only by one parent shares its parent's file, but initializes
only when the original dependency use executes.

### 0.a. Synthesize deferred initialization

Deliverable: reconstruct an eligible island as root-level lexical bindings plus
one guarded `_init()` function, independently of choosing its physical owner.

- Keep symbols at the containing ESM module's root. Move initialization
  assignments and effects into `_init()`, preserving their original order.
  Exported functions continue to close over those root bindings; do not return
  copied exports from a factory or recreate functions on each activation.
- Lower declarations to root-level `var` where appropriate. Preserving TDZ errors
  is not a requirement; do not introduce TDZ guards. Preserve binding identity,
  live reads, and other existing read/write restrictions.
- Emit per-island initialization state and cached failure. Mark initialization
  as entered before running its body. Preserve once-only success and failure,
  including `throw undefined`; do not inherit CJS retry-after-failure semantics.
- Start with synchronous, acyclic islands. Keep unsupported reentry, namespace
  observations, and top-level-await behavior on native module boundaries.
- Use local generated code by default. Share a minimal helper only if needed;
  ordinary modules remain unwrapped native ESM.

Regressions:

- Merely evaluating the containing file does not run deferred initialization.
- Root-level functions retain access to initialized private state and keep their
  identity. Include
  `const secret = makeSecret(); export function read() { return secret; }`.
- Repeated activation initializes once; failures remain cached for every thrown
  value without repeating effects.

### 0.b. Insert activation calls during segment reconstruction

Deliverable: connect original dependency uses to the generated initializers using
existing cached recipes and typed output requests.

- During reconstruction, replace each transformed async dependency use with
  asynchronous activation of its target, followed by the required binding or
  namespace result. Do not execute `_init()` synchronously while evaluating the
  original import expression.
- For a co-located target, call its local initializer. For a target in a library
  chunk, load that native ESM chunk, then call its exported initializer.
- Put synchronous dependency activation inside the dependent initializer, in
  original dependency order. Do not add guards before arbitrary exported
  function calls or binding reads.
- Resolve initializer references through final physical ownership. Compose
  inserted calls through existing recipe operations, preserving source-site
  guards, expression evaluation order, and exception handling.
- Retain observable namespace identity and live bindings where supported;
  unsupported observations keep their original native boundary. Repeated import
  promises need not be identical.

Regressions:

- Concurrent async uses share initialization and its outcome.
- Local and cross-chunk activation run at the original dependency use, not file
  evaluation time.
- Static dependencies initialize in order before the dependent body.
- An unreached guarded use does not activate its target; failures reject through
  the original import's exception context.

### 0.c. Apply single-parent placement

- Identify async targets with a single logical parent, accounting for all
  retained uses and public entry/export constraints.
- Assign eligible child segments to that parent's physical file.
- Reuse 0.a for deferred initialization and 0.b for dependency-use rewrites.
- Update typed requests and artifact membership after placement. Do not retain
  an unnecessary child file or fetch.

Regressions:

- The child requires no separate fetch, but its effects occur at the original
  dependency use.
- Unsupported targets retain their existing native placement and behavior.

Starting points: `crates/bundler/src/native_chunks.rs`,
`crates/bundler/src/native_segments.rs`, `crates/bundler/src/assembly.rs`, and
existing dependency/emission recipes. Features 1 and 2 reuse 0.a and 0.b rather
than introducing another activation mechanism.

## 1. Small-island library clustering

Goal: merge small segment islands with similar, potentially different
reachability masks into shared physical chunks.

- Use existing placement-compatible segment groups as islands and read their
  masks from `ChunkPlan.node_roots`; do not add another mask representation.
- Use one target size, `min_chunk_bytes`, measured in uncompressed UTF-8 bytes.
  Estimate size by summing retained recipe source-piece lengths; this is an
  approximation, not trial emission or a compressed-size prediction.
- Consider only nonempty-mask islands whose estimated size is below the target.
  Visit seeds in source-path/segment order.
- Rank compatible undersized candidates against the current library's union
  mask using intersection size divided by union size. Require nonzero overlap;
  break ties by source-path/segment identity.
- After each merge, update the union mask and estimated size. Stop once the
  library reaches the target or no compatible overlapping candidate remains.
  The final merge may overshoot; because candidates are individually undersized,
  the estimate remains below twice the target. Already-large islands are not
  merge candidates.
- Reuse deferred initialization from feature 0 where members have different
  activation boundaries. Co-location must not cause co-activation.
- Preserve individual logical reachability masks; change physical ownership and
  derive the library's availability from its members.
- Update artifact membership and generated requests to reference the merged file.

Starting points: `crates/bundler/src/chunks.rs` for existing masks, and native
placement/emission for grouping.

Regressions:

- Small islands with different overlapping masks share a file.
- Islands disjoint from the current library mask remain separate. For masks
  `{a}`, `{a,b}`, and `{b}`, the third island may join after the first two merge
  if the library is still undersized.
- Loading one member does not activate another.
- Shared bindings retain identity.
- Selection and output are deterministic.
- Threshold stopping, permitted overshoot, and exclusion of already-large
  candidates follow the specified rules.

## 2. Synthesize async-load boundaries

Goal: extract dependency islands behind generated async loads where all relevant
uses can accommodate suspension.

### 2.a. Cache async-deferred reference tags

Deliverable: tag existing references with whether they occur in an async function
body where suspension can be introduced. This step does not change emitted code.

- Add one source-local flag, `async_deferred`, to existing owned references.
  Existing binding identities and reference spans already identify the target
  and rewrite site; do not add function records or call-site tables.
- Track this flag during extraction. Entering a function body sets it from that
  function's async status, rather than inheriting the outer function's status.
  Restore the previous state on exit.
- Keep the flag false in parameter initializers and class field/static
  initialization, where an enclosing async function does not permit inserting
  await. A nested function body always establishes its own async status, even
  inside one of those initializers. Computed method keys retain their enclosing
  evaluation context.
- Carry the flag through checked graph references so cross-segment uses retain
  it. Preserve existing same-segment references; the tag does not itself create
  dependencies. Keep occurrence granularity through existing source-edge
  indices; do not combine flags for a shared consumer/target pair.
- Update cache schema/extractor identities and verify cache round-tripping of the
  flag. It describes syntax, not a persisted optimization decision.

Starting points: `crates/ast_segmenter/src/owned.rs`, existing reference-to-edge
construction in `crates/source_graph`, and cache serialization/validation in
`crates/segment_cache`.

Focused regressions:

- References in async bodies are tagged; module-level and synchronous-body
  references are not.
- Nested synchronous callbacks reset the flag; nested async functions set it.
- Parameter defaults and class initialization do not inherit the flag.
- Async arrows inside parameter defaults or class fields establish their own
  async bodies; computed method keys retain their enclosing context.
- Cross-segment references and cache reuse preserve the tag.

No function inventory, direct-await classification, call-graph solver, alias
analysis, or control-flow graph is needed for this step. The tag permits a new
suspension; it does not claim the reference already executes after an await.

### 2.b. Extract islands and introduce load boundaries

- Find retained functions and dependency islands used exclusively at eligible
  cross-segment reference sites tagged `async_deferred`. Inspect every retained
  value-use occurrence through existing checked binding/export identities;
  one synchronous use or unsupported public demand prevents extraction.
- Check captures, binding identity, receiver/argument evaluation, and control-flow
  guards. Initially leave unsupported closures, escapes, and synchronous
  consumers unchanged.
- Do not treat lexical containment in an async function as sufficient:
  synchronous nested callbacks cannot acquire suspension implicitly.
- Represent each generated boundary as a derived reference-site-to-segment-target
  record, using existing source coordinates. Do not fabricate source
  `DependencyId`s or modify cached source facts.
- Replace selected synchronous availability demands with generated async
  boundaries before computing final root reachability. Do not merely add async
  edges while retaining the eager path, and do not remove independently required
  source-module effects.
- Extend native async target/request handling beyond its current module-only
  target restriction. Resolve generated segment targets through final physical
  ownership, using typed artifact requests rather than fixed filenames.
- Initially support only reference forms that existing recipes can replace at
  their original evaluation position. Preserve shorthand property keys, guards,
  call receivers, argument/spread order, and surrounding exception handling.
  Reuse invocation and namespace recipes for supported larger replacements;
  leave unsupported forms unchanged.
- Emit native `import()` loads and suspension at selected use sites.
- Recompute exact reachability, then run parent-hoisting and library clustering.
  Keep generated boundaries as logical activation sites even when their bytes
  are subsequently co-located.

Starting points: source facts and recipes in `crates/ast_segmenter`, checked graph
edges in `crates/source_graph`, then bundler reachability and native emission.

Regressions:

- Before placement optimization, extracted code is absent from the initial
  synchronous reachability closure.
- A helper extracted from the same source module as its eager caller becomes a
  valid segment-target load without disturbing unrelated source-module effects.
- Cover both a separately fetched island and one subsequently parent-hoisted.
  Final placement may fetch bytes eagerly, but must preserve guarded activation
  and the introduced suspension; a separate download is not guaranteed.
- Guarded code loads only when its branch is reached.
- Calls preserve values, receivers, argument evaluation, and exception handling.
  Include `enabled && sink(first(), service.method(second()))`, shorthand object
  reads, optional calls, and load rejection inside `try/catch`.
- Mixed synchronous/asynchronous consumers prevent unsupported extraction.
- Added suspension is covered explicitly; physical merging does not erase it.

This feature requires an explicit opt-in contract: it introduces suspension and
load failures, and does not preserve exact interleaving with external work.
Use a build-level option, disabled by default. Added suspension may precede the
function's first original await and remains observable even when bytes are
already loaded. Load failures follow the original expression's exception
context. No timing analysis is required.

## Prior art and existing extension points

- Feature 0: study esbuild's
  [dynamic-import lowering](https://github.com/evanw/esbuild/blob/main/internal/js_printer/js_printer.go)
  and [initialization helpers](https://github.com/evanw/esbuild/blob/main/internal/runtime/runtime.go)
  for deferred activation and cached failures. Do not copy a general namespace
  runtime into this native emitter.
- Feature 1: study Rollup's
  [small-chunk merging](https://github.com/rollup/rollup/blob/master/src/utils/chunkAssignment.ts)
  and [minimum-size option](https://rollupjs.org/configuration-options/#output-experimentalminchunksize).
  Reuse structural grouping ideas, not an additional profitability model.
- Feature 2: Qwik's [optimizer](https://qwik.dev/docs/advanced/optimizer/)
  demonstrates lazy symbol extraction, but relies on explicit author boundaries
  and framework support; it does not establish eligibility for arbitrary
  async-body references.
- Extend existing exact masks, pure lexical-component placement, CJS availability
  separation, and occurrence-preserving graph edges. Reuse typed artifact
  requests and cached recipes rather than adding parallel representations.

## Delivery

- Land each feature as a separate logical change with focused native runtime
  fixtures and tests.
- Update affected crate README files and concise persistent design notes.
- Run targeted checks and `./scripts/spot-check` before committing.

## Out of scope

- Coverage collection and production-frequency estimates.
- Probability-based or profitability scoring.
- General closure extraction or synchronous-to-async call-chain conversion.
- A general module registry, binding-cell runtime, or dependency scheduler.
