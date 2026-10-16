# rspack Phase 0 research and fixture handoff

## Pins and licensing

- Immediate source: `web-infra-dev/rspack` at
  `96dcd72db4d25a89c37d22d514285ab82fc46a3d`.
  Source obtained through GitHub's contents/tree API using `gh`; no clone,
  dependency installation, or upstream scripts were needed.
- Independently inspected `webpack/webpack` at
  `e12e1224c2824c0cc78d6f3faf4cfa47926aa3cf` to verify inherited fixtures.
  This is a comparison pin, **not** the historical import commit.
- Both repositories identify MIT licenses. The actual notices, rather than
  repository metadata alone, were read and preserved:
  [rspack LICENSE][rspack-license], [rspack THIRD-PARTY-LICENSE][third-party],
  and [webpack LICENSE][webpack-license].
- `fixtures/bundler/rspack/{LICENSE,THIRD-PARTY-LICENSE,LICENSE.webpack}`
  match those upstream files byte-for-byte. The webpack MIT notice is
  independently applicable; rspack's main license does not supersede it.
  `fixtures/bundler/rspack/NOTICE` records file-level comparisons and
  copyright holders. Selected originals contain no additional per-file
  license headers. No unclear-license fixtures or dependencies were copied.

## Imported cases

All paths below are relative to upstream
`tests/rspack-test/normalCases/`. Each manifest records complete paths,
the full SHA, original test name, goals, and every adaptation.

| Local case | Original test/path | Preserved runtime observation |
| --- | --- | --- |
| `cjs-default-export` | `interop/exports/index.js`: “should exports default” | ESM default import is the CommonJS `module.exports = 42` value. |
| `cjs-named-reexport` | `parsing/harmony-commonjs/index.js`: “should be possible to reexport a module with unknown exports” | Named requests traverse an ESM `export *` facade to `exports.a`/`exports.b`. |
| `cjs-self-cycle` | `runtime/circular-dependencies/index.js`: “should load circular dependencies correctly” | A self-require returns the already assigned primitive `module.exports`, without recursing forever. |
| `dynamic-import-cjs` | `chunks/import/index.js`: “should be able to use import” | Dynamic import exposes the CJS primitive through `.default`; added trace observes the asynchronous boundary and evaluation before fulfillment. |
| `esm-live-binding` | `parsing/harmony-spec/index.js`: “should establish live binding of values” | Imported `value` changes from 0 to 2 after calling the exporter’s `add(2)`. |
| `esm-effect-order` | `parsing/harmony-spec/index.js`: “should execute modules in the correct order” | Effects are `a,b,c` despite `import "./order-b"` appearing after `log("c")` in the importer. |

General adaptations: replace assertion wrappers with deterministic console
observations; add explicit ESM request extensions; rename CommonJS dependencies
to `.cjs` where required by native Node's module classification; add a minimal
`files/package.json` with the declared module type. Original `index.js` entry
names are preserved. The `dynamic-import-cjs` trace is explicitly supplemental,
not an assertion claimed to exist upstream. Exact namespace-object shape is
excluded because Node versions and bundlers expose different metadata keys.

Webpack counterpart support files are byte-identical before adaptation.
The entry assertions match except for framework adaptation/test-name prefixes;
the inherited origin mapping is in `NOTICE`. `interop/exports` was verified
within rspack but is not attributed to an independently established webpack
antecedent.

## Structure and implications for the bundler plan

1. **Port behavior, not the upstream harness.** [Normal.test.mjs][normal-entry]
   recursively discovers normal cases, while [case/normal.ts][normal-processor]
   configures, compiles, runs, and checks them. Its [default options][normal-options]
   select `async-node`, enable export/side-effect analysis and optional
   concatenation, and configure non-Node resolution extensions/loaders.
   Running copied `index.js` files unchanged would therefore test an absent
   assertion/runtime environment, not portable JavaScript.
2. **Allocate CommonJS records before evaluation.** The
   [self-cycle source][cycle] replaces `module.exports` before requiring itself.
   A future module factory/cache needs the current replacement value, not just
   a stale initial `exports` object. This tiny case is deliberately a self-cycle;
   it does not claim multi-module SCC or complex partial-object coverage.
3. **Classify requests and export models separately.**
   [CJS default import][default-export] and
   [named requests through a facade][named-export] cannot share the naive
   “every import is an ESM named binding” path. Node's accepted static
   `exports.a`/`exports.b` forms establish only this case's baseline.
   Arbitrary dynamic CommonJS names, Babel `__esModule`, and requiring ESM need
   separate compatibility decisions; they are not inferred from this pass.
4. **Represent live bindings, not one-time reads.**
   [The native live-binding assertion][live-test] mutates the exported storage
   through an imported function. AST node identity or import-edge identity alone
   does not encode storage identity. Later incremental evaluation needs to
   distinguish changed module definitions from live runtime state.
5. **Keep initialization order distinct from source statement order.**
   [order-c.js][effect-source] puts an import after a logging statement;
   [the assertion][effect-test] still requires `a,b,c`. Dependency evaluation
   precedes the importer body, and effect-only imports must remain reachable.
   Segment-level pruning/reordering must preserve module-level initialization.
6. **An async edge is not a prescribed chunk layout.**
   [The dynamic-import test][dynamic-test] requires namespace delivery through
   a Promise. The local trace makes “synchronous caller, dependency evaluation,
   continuation” visible without testing chunk count, chunk names, or webpack's
   runtime. Chunk planning and emitted-bundle execution remain future drivers.

### Deliberately not imported

- `chunks/statical-dynamic-import` contains useful usage-analysis tests but
  depends heavily on `__webpack_exports_info__`, magic comments, request queries,
  and exact used-export observations. [Its assertions][engine-specific] are not
  a standalone runtime contract.
- The other `parsing/harmony-spec` tests exercise eval, transpiled ES5 live
  bindings, export cycles, and self-imports. They remain expansion candidates;
  this six-case corpus does not imply those features are covered.
- `async-modules/double-import` uses `await require("./main")` to consume
  a top-level-await module. [That harness behavior][async-require] is not
  native Node require-of-ESM behavior; silently turning it into a passing
  native test would overstate compatibility.
- No snapshots, bundler globals, external dependencies, package download
  scripts, optimization flags, or engine-specific output assertions are copied.

## Validation and integration

- All six manifests parsed and all six entries passed on Node `v22.22.2`,
  with exact expected stdout, empty stderr, and zero exit status.
  They were executed directly in their checked-in `files/` directories
  (read-only baseline; no temporary directory or generated runner).
- These are fixture-baseline results, **not bundler conformance results**.
  The shared runner and future bundler driver remain integration-owned.
- License copies were compared against GitHub's pinned source bytes.
- Every case preserves an `index.js` entry. Integration added a scoped
  `.gitignore` exception for those files and accepted all six cases through
  the shared temporary-directory runner. No `node_modules` directories were added.
- No crates, shared docs, `.gitignore`, runner, or commits were modified.
  Full `./scripts/spot-check` is left to integration; no build scripts from
  either upstream repository were executed.

[rspack-license]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/LICENSE#L1-L21
[third-party]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/THIRD-PARTY-LICENSE#L1-L26
[webpack-license]: https://github.com/webpack/webpack/blob/e12e1224c2824c0cc78d6f3faf4cfa47926aa3cf/LICENSE#L1-L20
[normal-entry]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/Normal.test.mjs#L1-L5
[normal-processor]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/packages/rspack-test-tools/src/case/normal.ts#L36-L70
[normal-options]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/packages/rspack-test-tools/src/case/normal.ts#L108-L211
[cycle]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/runtime/circular-dependencies/circular.js#L1-L2
[default-export]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/interop/exports/index.js#L1-L4
[named-export]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-commonjs/index.js#L25-L30
[live-test]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-spec/index.js#L8-L12
[effect-source]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-spec/order-c.js#L1-L4
[effect-test]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/parsing/harmony-spec/index.js#L26-L28
[dynamic-test]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/chunks/import/index.js#L1-L11
[engine-specific]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/chunks/statical-dynamic-import/index.js#L1-L11
[async-require]: https://github.com/web-infra-dev/rspack/blob/96dcd72db4d25a89c37d22d514285ab82fc46a3d/tests/rspack-test/normalCases/async-modules/double-import/index.js#L1-L4
