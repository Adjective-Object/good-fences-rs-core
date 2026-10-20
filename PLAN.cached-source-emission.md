# Cached source analysis and parse-free emission

## 1. Extract additional source-local facts

- Extend `ast_segmenter::owned::ModuleFacts`; reuse existing bindings, references, dependencies, spans, effects and invocations.
- Record statement kinds, declaration kinds per occurrence, and function/arrow containment; distinguish hoisted functions from later declarations.
- Record import/export syntax ranges and default forms: expression, named/anonymous function/class, declaration prefix, body and terminator.
- Record shorthand contexts, including destructuring and special `__proto__` keys.
- Record namespace-member occurrences and receiver-sensitive call/tag/write/delete contexts.
- Record observable function/class/name-inference sites and lexical capture constraints.
- Extend invocation facts with optional-chain steps, receiver structure and parentheses/guard boundaries.
- Record sourceURL/sourceMappingURL comment spans.
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
- Derive connector eligibility from statement kinds and retained content.
- Use declaration occurrences and function containment for hoisting/eager-read constraints.
- Evaluate name/capture compatibility from facts, not trial `body()` emission.
- Keep actual symbol demand, initialization, effect order and activation constraints derived.
- Preserve structural relocation tests and the SCC/await/connector timing regressions.

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
- Replace original unshadowed require sites through checked dependency IDs; never scan arbitrary strings or lexical shadows.
- Remove output-request discovery parses; retain collision/package-format validation.
- Preserve nested names, aliases, live/default exports and canonical namespace identity.

## 8. Remove hint scans; validate only final artifacts

- Consumers: `rewrite::{source_hint_edits, validate}` and all output assembly paths.
- Remove cached source-hint spans during source rendering; generators add none.
- Run parser/semantic validation once per final executable artifact, after naming/alias construction; skip intermediate candidates.
- Keep low-level caller-source verification; cache-backed `BuildSession` reuses hash-checked facts without reparsing original sources.
- Test generated-code errors remain explicit; no silent validation bypass.

## 9. Prove the migration and remove obsolete paths

- Consumers: `BuildSession` counters/fingerprints, benchmark, release checks and docs.
- Land independently reviewed consumer migrations in order; delete replaced parser paths rather than retaining a second backend.
- Count extraction, source/fragment reparses and final-artifact validation separately.
- Assert zero source/fragment reparses and one validation parse per final executable artifact; no-op builds perform neither.
- Compare cold/warm/edit benchmarks and cache size against a recorded baseline; report timings, not speedups inferred from parse counts.
- Run cache parity, structural/runtime regressions, `scripts/bundler-release-check` and `./scripts/spot-check`; update crate READMEs.
- Out of scope: minification, upstream parity and finer later-layer invalidation.
