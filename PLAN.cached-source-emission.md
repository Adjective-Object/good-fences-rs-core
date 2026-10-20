# Cached source analysis and parse-free emission

## 1. Extract additional source-local facts

- Extend `ast_segmenter::owned::ModuleFacts`; reuse existing bindings, references, dependencies, spans, effects and invocations.
- Cache a compact per-segment `connector_only` flag: imports, declaration-free reexports/export lists, or empty statements; not merely effect-free code.
- Record declaration hoisting per occurrence and function/arrow containment; distinguish hoisted functions from later declarations.
- Record import/export syntax ranges and default forms: expression, named/anonymous function/class, declaration prefix, body and terminator.
- Record shorthand contexts, including destructuring and special `__proto__` keys.
- Record namespace-member occurrences and receiver-sensitive call/tag/write/delete contexts.
- Record observable function/class/name-inference sites and lexical capture constraints.
- Extend invocation facts with optional-chain steps, receiver structure and parentheses/guard boundaries.
- Record sourceURL/sourceMappingURL comment spans; use compact flags/indices and existing spans instead of generic AST or redundant classification records.
- Keep records owned, source-ordered and in original UTF-8 coordinates; never cache chosen names, purity decisions, resolved targets or chunk owners.

## 2. Persist and validate the new facts

- Consumer: `segment_cache::{CachedModule, CacheRecord}` and `validate.rs`.
- Bump schema for the wire change and extractor identity for changed extraction.
- Validate ranges, containment, ordering, local IDs and cross-record links on
  encode/decode; retain bounded CBOR preflight and source/configuration hashes.
- Add arena-drop roundtrips, stale-record rejection and malformed-fact tests.
- Preserve opaque CJS bodies; do not persist graph/SCC/async dependency state.

## 3. Replace placement-analysis parses

- Consumers: connector detection in `native_chunks.rs`, `Placement::new` in `native_segments.rs`, and name-compatibility checks.
- Replace AST connector detection with cached `connector_only` flags; preserve the current all-source-segments classification initially.
- For retained-only classification, reduce flags over surviving segments; keep namespace, cycle and evaluation-order checks in the planner.
- Use declaration occurrences and function containment for hoisting/eager-read constraints.
- Evaluate name/capture compatibility from facts, not trial `body()` emission.
- Keep actual symbol demand, initialization, effect order and activation constraints derived.
- Preserve structural relocation tests and the SCC/await/connector timing regressions.

```rust
// Source-local classification is cached; context-dependent merge safety is not.
let connector = segments.iter().all(|segment| segment.connector_only);
```

## 4. Emit original segments without re-extraction

- Consumers: `native.rs::{body, retained_source, Syntax}`, ungrouped emission in `native_chunks.rs`, and `rewrite::{Shorthand, rename_private}`.
- Select original segments; address declarations/references by original binding IDs, never reconnect reparsed bindings by name.
- Use cached shorthand/default/export contexts to build checked source edits.
- Compose nested PURE/name/namespace/dependency edits before rendering; a replacing parent owns child edits explicitly.
- Preserve name inference, nested shadows, live exports, read-only imports and TDZ.
- Delete retained-source `parse_module`, its second AST parse and trial-render parses.

```text
source + cached facts + retained segments + derived edits -> rendered chunk
```

## 5. Lower PURE expressions from cached invocation structure

- Consumers: `emit.rs::{crosses_await_guard, pure_chain, chain}`.
- Replace callee-expression parsing with traversal of owned optional-chain facts.
- Preserve receiver/getter evaluation, argument order, spreads, parentheses and guards.
- Keep annotation policy/value demand derived; do not cache unconditional purity.
- Render nested source slices through the same edit composer as segment emission.
- Retain optional-chain, nested PURE and await/for-await regressions.

## 6. Prune chunks before rendering strings

- Consumer: `native_chunks.rs::prune_empty`.
- Track executable content, native namespace boundaries and ordered dependencies in the physical chunk plan, not emitted ASTs.
- Splice empty private jobs using that plan; preserve cyclic evaluation barriers.
- Project `members`, `modules`, `chunk_artifacts`, entries and async routes to survivors.
- Remove the emitted-source parse used to discover imports and empty bodies.
- Verify actual runtime order and that every metadata/request target exists.

## 7. Resolve output names through structured requests

- Consumers: `entry_outputs.rs::{apply, OwnedRequests}` and ESM/CJS emitters.
- Record generated requests as typed targets: static import/reexport, dynamic import, CJS require and namespace/export surfaces.
- Choose public filenames and aliases before rendering; distinguish ESM URL encoding from CJS filesystem paths.
- Assign artifact identities, destinations and aliases on interim structures; render source only after pruning and naming finish.
- Replace original unshadowed require sites through checked dependency IDs; never scan arbitrary strings or lexical shadows.
- Remove output-request discovery parses; retain collision/package-format validation.
- Preserve nested names, aliases, live/default exports and canonical namespace identity.

## 8. Remove redundant source verification parses

- Consumers: `link::esm`, `emit::checked_source` and the cache-backed `BuildSession` adapter.
- Today `link::esm` accepts a graph plus separate source strings and reparses those strings to compare facts/ranges.
- Carry validated source hashes and effective parser configuration with the extracted source records used to construct the graph.
- Verify source bytes match that provenance without parsing; reuse owned facts/ranges for both public and session emission.
- Preserve source/configuration mismatch errors, JavaScript support checks and graph/plan revision checks.
- Delete the re-extraction callback split; callers must not supply unrelated text with otherwise plausible byte ranges.

## 9. Remove hint scans; debug-validate final artifacts

- Consumers: `rewrite::{source_hint_edits, validate}` and all output assembly paths.
- Remove cached source-hint spans during source rendering; generators add none.
- Gate final parser/semantic validation with `#[cfg(debug_assertions)]`; release emission performs no parsing.
- In debug builds, validate once per final executable artifact after naming/alias construction; skip intermediate candidates.
- Keep source/cache integrity, edit-range/overlap, artifact-target and output-path validation enabled in every build.
- Test debug diagnostics and release parse counts explicitly; runtime regressions exercise generated output independently.

## 10. Prove the migration and remove obsolete paths

- Consumers: `BuildSession` counters/fingerprints, benchmark, release checks and docs.
- Land independently reviewed consumer migrations in order; delete replaced parser paths rather than retaining a second backend.
- Count extraction, source/fragment reparses and final-artifact validation separately.
- Assert zero source/fragment reparses; debug builds validate each final executable once, release builds never parse emitted output.
- Verify no-op builds perform neither and exercise both profiles in the migration gates.
- Compare cold/warm/edit benchmarks and cache size against a recorded baseline; report timings, not speedups inferred from parse counts.
- Run cache parity, structural/runtime regressions, `scripts/bundler-release-check` and `./scripts/spot-check`; update crate READMEs.
- Out of scope: minification, upstream parity and finer later-layer invalidation.
