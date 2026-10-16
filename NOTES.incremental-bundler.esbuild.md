# esbuild grounding and STEP0 fixtures

## Provenance and licensing

- Authorized peer checkout: `/home/adjective/Projects/esbuild`; inspected before
  cloning/reusing; clean after research. No upstream installation/build scripts ran.
- Exact HEAD: `f6058f8364fe7ab91ca57a83e02577ed74c9cae4`.
- Repository: <https://github.com/evanw/esbuild>.
- Verified [LICENSE.md][license]: MIT, Copyright (c) 2020 Evan Wallace.
  `fixtures/bundler/esbuild/LICENSE` is byte-identical, including the full notice.
- All six cases derive from inline file maps in
  `scripts/end-to-end-tests.js`, not emitted snapshots. The source file and
  selected maps have no additional per-file copyright/license notices.
  No third-party test corpora or `internal/xxhash` code were copied.
- Each `case.json` records repository, full SHA, original source path/test,
  license, and adaptations under the shared interchange schema.

## Architecture evidence for the planner

All references below are pinned to the inspected HEAD, not a moving branch.

- **Scan/parse/link separation:** [bundler.go:2176–2250][scan] consumes parse
  results, resolves import records to source indices, schedules dependencies,
  and distinguishes external edges. [js_parser.go:23–35][parser] documents two
  passes: AST/scope/symbol declaration, then binding/folding/lowering.
  [js_ast.go:1740–1791][parts] stores top-level statement parts, declaration/use
  facts, import record indices, part dependencies, and removability.
- **Link order matters:** [linker.go:227–350][link] clones the graph, scans
  imports/exports, computes liveness/reachability, computes chunks and cross-chunk
  dependencies, then generates output. [linker.go:1467–1550][exports] propagates
  dynamic export status before resolving star exports and matching imports.
  [linker.go:1762–1765][bind] makes import uses depend on declaring parts.
  Do not substitute a raw import graph for binding-level dependencies.
- **Two effect levels:** [graph/input.go:49–80][effects] conservatively defaults
  files to `HasSideEffects`, distinguishing package metadata, empty ASTs, and
  pure data loaders. Parts separately carry `CanBeRemovedIfUnused`.
  [linker.go:3220–3275][shake] retains mandatory effects and follows ordinary
  import effects unless metadata allows omission; `IgnoreDCEAnnotations`
  overrides those hints. Keep effect provenance distinct from symbol demand.
- **PURE is not blanket purity:** [js_parser.go:4232–4262][pure] attaches the
  annotation to the next call/new expression when annotations are enabled.
  [js_ast_helpers.go:2532–2557][purecheck] permits removing the call only if its
  arguments are also removable; [js_ast_helpers.go:812–824][pureargs] preserves
  argument effects when simplifying an unused annotated call, including spreads.
  Preserve argument evaluation; do not label the entire statement effect-free.
- **Typed async edges:** [ast.go:15–40][kinds] distinguishes import statements,
  `require`, dynamic imports, and `require.resolve` (also CSS kinds).
  [graph.go:126–202][clone] clones mutable link state and, with splitting enabled,
  promotes resolved dynamic import targets to additional entrypoints.
  [linker.go:3285–3290][asyncboundary] treats a dynamic import into a distinct
  entrypoint as a splitting boundary. Async evaluation is not just another
  eager reachability edge.
- **Reachability and grouping:** [linker.go:3148–3218][reach] computes liveness
  first, then per-file entrypoint bitsets, walking import records and part
  dependencies while respecting dynamic boundaries.
  [linker.go:3875–3992][chunks] groups live JS files by identical entry bitsets,
  creates even empty entry chunks, and sorts keys for determinism.
  This implementation groups files; its part-level liveness is not evidence
  that it currently emits an arbitrary independent chunk per statement.
- **CJS interop and initialization:** [linker.go:1900–1957][interop] marks
  wrapper/namespace conversions according to import kind and module format.
  [runtime.go:170–211][init] caches ESM initialization and CJS exports, allocates
  the CJS exports object before executing the body (supporting cycles), and
  distinguishes ESM cached failures from CJS reset-on-failure.
  [runtime.go:213–269][namespace] uses getters for export forwarding; `__toESM`
  handles Node-mode/default versus `__esModule` behavior, and `__toCommonJS`
  adds the compatibility marker. Concatenation alone is not sufficient.
- **Incremental boundary:** [cache_ast.go:152–189][cache] caches parsing by source
  path plus exact source/options equality, including diagnostics. Linking clones
  mutable facts. Borrow that separation, not a claim of fine-grained incremental
  linking: stable source identity, cached parse facts, and recomputed link state
  are distinct responsibilities. Future invalidation must account for changed
  bindings, effects, entry reachability, and cross-chunk dependencies.

## Selected portable cases

All paths are under `fixtures/bundler/esbuild/`; the names below are directory
names. Source links point to the actual original inline fixtures.

| Case | Upstream lines | Portable contract |
| --- | --- | --- |
| `shared-entrypoint-identity` | [8213–8233][shared] | Three entrypoints re-export shared/distinct objects; combined driver observes `false false true true`. |
| `dynamic-import-tla-order` | [8941–8969][tla] | Dynamic import and awaited re-export evaluation produce `123 1,2,3,4,5,6`. |
| `esm-sync-async` | [8136–8155][esmcase] | Static and dynamic ESM imports expose `123 -123`. |
| `cjs-sync-async-once` | [8157–8176][cjscase] | Static namespace/dynamic default CJS interop gives `123 -123`; explicitly added evaluation logging occurs once. |
| `unused-import-side-effects` | [4694–4699][sidecase] | Unused namespace import still evaluates the dependency's assignment; preserves `sideEffects: true`. |
| `reexport-cycle` | [1894–1912][cyclecase] | Cyclic functions through a star re-export retain identities (`false true`). Original `.ts` bodies are already plain JS. |

Harness changes are explicit per case: native ESM package scopes/extensions,
source paths instead of `out/`, console observations instead of throw/assert
checks, and direct async execution instead of the upstream harness callback.
Only the CJS once-only log is an added semantic observation, not an upstream
assertion. All cases require only Node; `globalThis` is standard JavaScript.

## Validation and limits

- Validated on Node `v22.22.2`: six manifests, nine separate Node invocations,
  exact stdout matches, zero stderr, successful exit status, and byte-identical
  MIT notice. Executed source files directly without writing scratch files.
- `shared-entrypoint-identity` executes `a.js`, `b.js`, `c.js`, then `node.js`
  in separate processes as specified by the interchange. The first three are
  silent; only the driver checks all identities together in one process.
- This is a **native-runtime baseline, not bundler conformance**. No bundler,
  runner, Rust crate, shared design document, or generated output was added.
  No commit was created; combined validation/spot-check belongs to integration.
- Intentionally omitted output/chunk names, hashes/counts, snapshots, lowering,
  CSS imports, package dependencies, plugin behavior, engine-specific harnesses,
  forced `require` wrappers around ESM, and mixed-format cyclic bundle tests.
- Native Node ignores `sideEffects` and PURE hints. Upstream “remove unused”
  tests that intentionally suppress observable source effects cannot be treated
  as native-runtime expectations. Add explicit bundler-only drivers for those,
  export diagnostics, linker behavior, chunk grouping, and incremental edits.
- TLA is included as a future async-evaluation contract, not a requirement that
  an initial synchronous bundler phase already implement it. Early phases should
  reject/defer unsupported constructs explicitly. No blockers for STEP0.

[license]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/LICENSE.md
[scan]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/bundler/bundler.go#L2176-L2250
[parser]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_parser/js_parser.go#L23-L35
[parts]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_ast/js_ast.go#L1740-L1791
[link]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L227-L350
[exports]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L1467-L1550
[bind]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L1762-L1765
[effects]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/graph/input.go#L49-L80
[shake]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3220-L3275
[pure]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_parser/js_parser.go#L4232-L4262
[purecheck]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_ast/js_ast_helpers.go#L2532-L2557
[pureargs]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/js_ast/js_ast_helpers.go#L812-L824
[kinds]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/ast/ast.go#L15-L40
[clone]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/graph/graph.go#L126-L202
[asyncboundary]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3285-L3290
[reach]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3148-L3218
[chunks]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L3875-L3992
[interop]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/linker/linker.go#L1900-L1957
[init]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/runtime/runtime.go#L170-L211
[namespace]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/runtime/runtime.go#L213-L269
[cache]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/internal/cache/cache_ast.go#L152-L189
[shared]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L8213-L8233
[tla]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L8941-L8969
[esmcase]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L8136-L8155
[cjscase]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L8157-L8176
[sidecase]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L4694-L4699
[cyclecase]: https://github.com/evanw/esbuild/blob/f6058f8364fe7ab91ca57a83e02577ed74c9cae4/scripts/end-to-end-tests.js#L1894-L1912
