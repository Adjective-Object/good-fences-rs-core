# Cached source analysis and parse-free emission

## 1. Precompute source-local emission recipes and constraints

- Extend [`ModuleFacts`](crates/ast_segmenter/src/owned.rs#L131); reuse existing bindings, references, dependencies, spans, effects and [invocations](crates/ast_segmenter/src/invocation.rs#L45).
- Cache a compact per-segment `connector_only` flag: imports, declaration-free reexports/export lists, or empty statements; not merely effect-free code. Current derivation: [`native_chunks.rs:65`](crates/bundler/src/native_chunks.rs#L65).
- Precompute source-local placement constraints: [hoisted availability](crates/bundler/src/native_segments.rs#L60), [eager initialization and write co-location](crates/bundler/src/native_segments.rs#L111), and [name/capture restrictions](crates/bundler/src/native_segments.rs#L515).
- Compile compact per-segment emission recipes while the AST and semantic scopes are available; parameterize names, targets and optional transforms by local IDs. Replaces [`body`](crates/bundler/src/native.rs#L255) and [edit composition](crates/bundler/src/native.rs#L410).
- Bake [shorthand expansion](crates/bundler/src/rewrite.rs#L209), [export-wrapper removal](crates/bundler/src/native.rs#L509) and [default initialization/hoisting](crates/bundler/src/native.rs#L530) into recipes; reuse existing spans rather than duplicating them.
- Preclassify namespace accesses as [direct-read candidates](crates/bundler/src/native.rs#L586) or native-namespace uses, preserving [receiver/tag/write/delete semantics](crates/bundler/src/native.rs#L609).
- Precompute [name-sensitive rewrite restrictions](crates/bundler/src/native.rs#L560) instead of making consumers rediscover inference sites and [scope capture](crates/bundler/src/native_segments.rs#L573).
- Compile PURE lowering alternatives: [receiver, argument/spread and optional-chain handling](crates/bundler/src/emit.rs#L324), including [parentheses/guard boundaries](crates/bundler/src/emit.rs#L306) and [await restrictions](crates/bundler/src/emit.rs#L279).
- Include [source-hint removal](crates/bundler/src/rewrite.rs#L94) in recipes; store sparse operations/indices, not a serialized AST, duplicate source or expanded output variants.
- Keep records [owned and source-local](crates/segment_cache/src/lib.rs#L114), source-ordered and in original UTF-8 coordinates; never cache chosen names, purity decisions, resolved targets or chunk owners.

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
- Consume precomputed source-local availability, initialization and write constraints; resolve cross-file endpoints through checked symbols.
- Check candidate names against precomputed capture/name restrictions, not syntax traversal or trial `body()` emission.
- Keep actual symbol demand, initialization, effect order and activation constraints derived.
- Preserve structural relocation tests and the SCC/await/connector timing regressions.

```rust
// Source-local classification is cached; context-dependent merge safety is not.
let connector = segments.iter().all(|segment| segment.connector_only);
```

## 4. Instantiate cached segment emission recipes

- Consumers: `native.rs::{body, retained_source, Syntax}`, ungrouped emission in `native_chunks.rs`, and `rewrite::{Shorthand, rename_private}`.
- Select retained recipes and supply chosen symbol names, resolved targets and policy-dependent transform choices.
- Copy unchanged original source slices; instantiate typed replacement slots using binding/dependency IDs, never name-based reconnection.
- Use precomputed rewrite nesting/composition; do not rediscover shorthand, scope, export forms or edit containment during emission.
- Preserve original-coordinate edit validation; a replacing parent explicitly owns child operations.
- Preserve name inference, nested shadows, live exports, read-only imports and TDZ.
- Delete retained-source `parse_module`, its second AST parse and trial-render parses.

```text
cached recipe + original source + link-time substitutions -> rendered chunk
```

## 5. Select precomputed PURE lowering alternatives

- Consumers: `emit.rs::{crosses_await_guard, pure_chain, chain}`.
- Replace callee-expression parsing and chain-shape reconstruction with cached lowering recipes.
- Preserve receiver/getter evaluation, argument order, spreads, parentheses and guards.
- Keep annotation policy/value demand derived; do not cache unconditional purity.
- Select the retained-call or eligible lowered form; fill names and nested source slots through the segment recipe renderer.
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
- Apply precomputed source-hint removal operations during rendering; generators add none.
- Gate final parser/semantic validation with `#[cfg(debug_assertions)]`; release emission performs no parsing.
- Before gating, enforce parse-free identifier legality for rename targets, including reserved words and strict-mode restrictions; reuse `symbols.rs` rules.
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
- Out of scope: minification, source-map support/out-of-segment source-map hint cleanup, upstream parity and finer later-layer invalidation.
