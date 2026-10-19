# Portable bundler semantic fixtures

These small upstream adaptations and locally authored cases describe observable
JavaScript runtime semantics. One runner validates the inputs using Node, builds
through the public Rust library/publisher, and compares emitted observations.

From the repository root, with Node.js 22 and the repository Rust toolchain:

```sh
cargo build -p bundler --example build
node scripts/bundler-fixtures.mjs
```

After building the driver, no package installation, upstream engine, or network
access is needed. `yarn test:bundler-fixtures` builds the driver and runs the same
command.
An optional positional argument selects a different corpus root. The runner
fails for missing/empty corpora, malformed or unsupported metadata, execution
errors, timeouts, stderr, or output differences; it never silently skips cases.
Input, build, required-backend and output failures are reported separately.
Bad inputs are not built; unexpected failures do not stop later valid cases.
TTY output colors `PASS` green, `FAIL` red, and `backend=legacy` red even on
passing cases. `FORCE_COLOR=1` enables color in redirected output; `NO_COLOR`
disables it (taking precedence). Structured API results remain uncolored.

The corpus contains **30 cases / 41 entry executions**: six esbuild cases,
six rspack cases and eighteen authored cases. The upstream cases cover
shared entrypoint identity, static/dynamic imports, CJS default/named interop
and cycles, ESM live bindings, re-export cycles, and evaluation order.
Authored cases cover TDZ and const-write failures with preceding effects,
hoisting, default snapshots/live aliases, inferred names, `__proto__` shorthand
and destructuring, reverse-path static import ordering, namespace observations,
ambient deferred effects and once-only shared identity, native TLA sibling/SCC
jobs, cached rejection/TDZ, dynamic exported-then assimilation, and native CJS
ambient activation/identity, failure retry and mixed-cycle error contracts.
Rspack-derived webpack fixtures retain webpack's notices as well as rspack's.
The conformance CI job builds the driver and runs this corpus and the runner's
tests.

## Layout and manifest

```text
<group>/LICENSE                  # required when upstream provenance is present
<group>/<case>/case.json
<group>/<case>/files/entry.js
```

Every case has `id`, `moduleType`, `entryPoints`, `expectations` and `goals`.
Only `upstream` is optional at the top level; no unknown fields are accepted.
An upstream adaptation looks like:

```json
{
  "id": "upstream/descriptive-case",
  "upstream": {
    "repository": "https://github.com/owner/repo",
    "commit": "0123456789abcdef0123456789abcdef01234567",
    "paths": ["path/to/original/test"],
    "test": "original test name",
    "license": "MIT",
    "adaptations": ["Replaced the upstream assertion harness with stdout observations."]
  },
  "moduleType": "module",
  "entryPoints": ["entry.js"],
  "expectations": {
    "stdout": ["deterministic output line"]
  },
  "goals": ["Semantic properties exercised by the case."]
}
```

- `id` matches the slash-separated case directory relative to the corpus root.
- Omit `upstream` for locally authored cases, as in `authored/`. No upstream
  license is inferred or required for them. If `upstream` is present, the entire
  provenance object and nonempty group `LICENSE` are still mandatory; all twelve
  existing upstream cases retain their pinned metadata and licenses unchanged.
- `upstream.repository` is the canonical HTTPS GitHub repository URL;
  `commit` is its full 40-digit hexadecimal revision. `paths` is a nonempty list
  of repository-relative original files, and `test` identifies the original test.
- `license` records the upstream license identifier/expression. Preserve the
  complete upstream license in `<upstream>/LICENSE` and applicable source
  copyright/license notices. Record any additional third-party licensing.
  Only import material with verified redistribution rights.
- `adaptations` lists every harness, source, or semantic change, including
  replacing assertions, removing engine-specific configuration, renaming files,
  and reducing a larger upstream test. An empty list means no adaptations.
  Metadata validation checks structure, not historical or legal accuracy:
  importers must verify provenance and licensing against the pinned upstream.
- `moduleType` is `module` or `commonjs`. If `files/package.json` is absent,
  the runner creates `{ "type": "<moduleType>" }` in the execution copy.
  Existing package metadata is preserved, including its own `type`.
- `entryPoints` is a nonempty ordered list of unique files relative to `files/`.
  All manifest paths use `/`, without absolute paths, backslashes, drive prefixes,
  empty components, `.` or `..`. Symlinks and nonregular source files are rejected.
- `expectations.stdout` is an ordered list of strings without line terminators;
  it may be empty for a deliberately silent contract. Output from entrypoints is
  concatenated in manifest order. LF and CRLF delimit lines; the final line
  terminator is not an extra line, but actual blank lines and whitespace matter.
- `goals` is a nonempty list of semantic properties; it is descriptive metadata,
  not an additional assertion language.

### Expected runtime failures and backend requirements

`expectations` requires `stdout` and optionally accepts only `errors` and
`backend`. For example, this is the complete authored TDZ manifest:

```json
{
  "id": "authored/native-tdz",
  "moduleType": "module",
  "entryPoints": ["entry.js"],
  "expectations": {
    "stdout": ["before TDZ"],
    "errors": {"entry.js": {"name": "ReferenceError"}},
    "backend": "native"
  },
  "goals": ["Preserve an observable effect before a native lexical TDZ failure."]
}
```

`errors`, if present, is a nonempty map keyed only by declared entrypoints.
Each value requires a nonempty string `name` and optionally a nonempty string
`code`; no stack, message, or other fields are accepted. `name` is the thrown
Error's name, or JavaScript `typeof` for a non-Error throw. Entries absent from
the map must succeed. Both synchronous throws and top-level-await rejections
are observed; preceding stdout must still match exactly. A wrong name/code,
missing throw, unexpected stderr, nonzero exit, signal, timeout or output overflow
always fails. Parse errors in expected-failure source cases are rejected by
Node syntax checks; build failures are never accepted as runtime failures.

The only accepted `backend` requirement is `"native"`. The runner checks the
actual production selection and reports a backend failure if such a case falls
back to legacy. All thirty current cases require native. Legacy reporting and
rejection remain useful when testing older driver binaries; production emission
has no legacy backend.

Sources must be self-contained JavaScript and local data. No external packages,
`node_modules`, project sources, downloads, Jest, webpack/rspack globals, or
package lifecycle scripts are supported. The runner never installs dependencies
or invokes scripts; missing runtime dependencies fail execution. Node built-ins
are available. Only trusted, reviewed fixtures should be run: Node execution is
**not a sandbox**, and this runner does not enforce a network or filesystem jail.

## Unified execution and reporting

Cases are discovered in deterministic ID order and all manifests are validated
before execution. Each case's `files/` tree is copied into a fresh managed
directory under project-local `target/p1-test-tmp` by default, with separate
source-observation and build copies. Each entrypoint runs in a separate Node
process with that directory as its cwd,
in manifest order. Entrypoints share the case's copied filesystem, not process
state. The runner removes the copy in `finally` on success and failure.
Node preload/search and forced-color overrides are removed and coverage is
disabled. Each entrypoint has a five-second timeout and a 1 MiB output-buffer limit.

For both successful and expected-failure entries, one shared ESM observation
driver imports the absolute entry path through Node's normal file loader.
Completion (null or normalized error) travels over a separate pipe, never stdout
or stacks. Premature exit, including exit zero, cannot count as completion.
This observes imported module execution, not CLI-only `require.main` semantics.
It uses no preload or custom loader hooks. Fixture code remains trusted, not jailed.

The runner builds the independent copy and executes its resulting entrypoints,
comparing the same per-entry stdout and normalized error observations.
Explicit unsupported capabilities are visible, never counted as passes. Linking,
chunking, diagnostics, incremental invalidation, and bundler-only behavior need
their own drivers and expectations. Chunk names, chunk counts, generated-code
snapshots, and incidental formatting are not this corpus's semantic contracts.

Runner regression tests use synthetic fixtures only:
`node --test scripts/bundler-fixtures.test.mjs scripts/bundler-fixtures-build.test.mjs`.

## Public-pipeline results (Node 22)

The single command above validates manifests/provenance first and compares
manifest expectations with source and emitted per-entry stdout and normalized
runtime outcomes through the shared observer.
All **30 contracts / 41 emitted entry executions** pass, with zero
unsupported skips. The actual backend is reported for every built case:

- **Native (30, all explicitly required):** all eighteen authored cases,
  `esbuild/unused-import-side-effects`, `rspack/esm-effect-order`,
  `rspack/esm-live-binding`, `esbuild/shared-entrypoint-identity`,
  `esbuild/reexport-cycle`, `esbuild/dynamic-import-tla-order` and
  `esbuild/esm-sync-async`, `esbuild/cjs-sync-async-once`, `rspack/cjs-default-export`,
  `rspack/cjs-named-reexport`, `rspack/cjs-self-cycle` and
  `rspack/dynamic-import-cjs`. The last five are graduated native CJS coverage.
- **Legacy (0), unsupported (0):** the production legacy backend is deleted.
  Rust Node regressions cover low-level root shapes outside the portable public
  driver API. No upstream parity is claimed.

The public driver also returns the plan's effect-order-cycle diagnostics in
`planWarnings`. These must exactly match the build warnings and be known cycle
diagnostics; unrelated cache/publication/unknown warnings still fail. Results
retain nonempty `warnings`, and the CLI prints them as `WARN` lines. This keeps
async SCC planning diagnostics visible without treating the scalar DAG as an
executor or suppressing native binding failures.

### Authored migration baseline

Nine cases port representative inline Rust regressions through the same unbundled
and public-build observer; those migrated cases each have one entry. The added
barrel-surface regression has three entries: the public reexport entry, its full
dependency, and a static namespace observer. Outcomes below are observed, not
linker-shape assertions. No upstream provenance is fabricated for local inputs.

| Added case (`authored/`) | Actual backend | Expected completion / observations |
| --- | --- | --- |
| `native-hoisting` | native, required | Success; named default function hoisting, undefined early vars, repeated var initialization, live barrel mutation |
| `native-default-bindings` | native, required | Success; expression effects in order, snapshot `1`, mutated bindings `2` / `4` |
| `native-name-inference` | native, required | Success; declaration/arrow names, default function/class names and descriptors, unnamed comma/conditional forms |
| `native-proto-destructuring` | native, required | Success; literal/escaped own keys, prototype setter distinction, binding/assignment defaults |
| `native-static-order` | native, required | Success; `z` before `a` through a pure barrel, source names, free global and strict `this` |
| `native-const-write` | native, required | Uncaught `TypeError` after caught TDZ/const errors, imported value and preceding effect |
| `native-barrel-surface` | native, required | Success; filtered public export keys, default-star exclusion, no false star ambiguity when the full dependency is another public entry |
| `namespace-observations` | native, required | Success; keys/descriptors, computed read, live aliases, identity, method receiver, read-only write |
| `ambient-dynamic-laziness` | native, required | Success; zero effects before import, outer then deferred inner effects, cached namespace identity |
| `ambient-once-identity` | native, required | Success; one shared ambient initialization across concurrent consumers, object/function/namespace identity and mutation |
| `native-async-order` | native, required | Success; `a-before,b,a-after,job` sibling interleaving, deferred outer/inner effects and repeated namespace identity |
| `native-async-cycles` | native, required | Hoisted cyclic functions and suspension; uncaught cycle `ReferenceError`; shared cached rejection across concurrent imports and retries |
| `native-dynamic-then` | native, required | Native microtasks/exported-then assimilation, exact namespace keys, default snapshot and live mutation |
| `native-cjs-activation` | native, required | Shared static/named/dynamic/require identity, ambient once-only effects, deferred require and wrapper this/exports replacement |
| `native-cjs-retry` | native, required | Concurrent/repeated dynamic facade failures share a cached error; a later require retries the failed factory |
| `native-cjs-cycles` | native, required | Legal CJS-first require through ESM, shared partial-object identity, and ESM-first/direct mixed-cycle `ERR_REQUIRE_CYCLE_MODULE` outcomes |

The existing `authored/native-tdz` remains an uncaught `ReferenceError` contract
after `before TDZ`; `authored/runtime-outcomes` now requires native with successful,
throwing and rejecting entries. Errors constrain names, not stacks or messages;
these new cases need no fabricated error codes. Ambient cases call functions
through `globalThis` and observe effects before/after dynamic activation; they do
not inspect source counts, generated code, chunks or import closures.

Four redundant inline Rust tests are retired; exact mappings and reasons for
retaining broader/private-API tests are in
[`NOTES.incremental-bundler.md`](../../NOTES.incremental-bundler.md).
`scripts/bundler-release-check` runs the unified corpus command once, so these
contracts remain release-gated. Every subsequent linker stage must either
graduate applicable legacy fixtures to required native coverage, add meaningful
regressions for its new behavior, or both. Compare actual reported backends with
this baseline; a legacy pass alone is not progress on native support.

These observations do not establish general bundler conformance. It additionally
executes an authored same-runtime concurrent multi-entry CJS/ESM identity case
and four bundler-only source-range diagnostic cases (nonliteral import,
unconfigured external, require-of-async-ESM, TS emission).
No upstream files, provenance or licenses are changed.

`runFixtures(root, {driver, tempRoot, timeoutMs, unsupported})` is the single
programmatic harness API. `unsupported` must explicitly map known case IDs to
nonempty reasons; results say `unsupported`, with zero emitted executions, never
`passed`. Unexpected build failures remain failures, not inferred skips.
Passed results contain `{id, status: "passed", backend: "native" | "legacy",
executions, stdout}`. Failed results contain `{id, status: "failed", stage,
executions: 0, error}`, with the actual `backend` when the build succeeded.
`stage` is `input` (source observations/syntax), `build` (build/publication/artifact
checks), `backend` (required-native mismatch), or `output` (emitted runtime or
observation comparison). `error` retains the original error and its causes.
Execution totals count entries in passing cases. Malformed metadata throws an
`INPUT metadata` error before any source runs, rather than executing a partially
valid corpus. Invalid runner configuration/infrastructure errors throw instead
of becoming fixture passes. The CLI prints all case results and native/legacy
pass, failure, unsupported and execution totals, then exits nonzero for any
unexpected failure. Fatal runner errors include causes in diagnostics.
The shared `observeEntries` helper returns `{stdout, outcomes}`, where each ordered outcome
is `{entry, stdout, error}` and `error` is null or `{name, code?}`. Differential
comparison includes observed codes even when the manifest does not constrain
them, and compares stdout per entry as well as the manifest's concatenated lines.
`buildProject` invokes `bundler`'s `examples/build.rs` library driver; there is no
competing JS resolver/graph/emitter or handwritten CLI framework.

Managed copies default to project-local `target/p1-test-tmp`. Runtime invocations
have five-second timeouts, bounded output, sanitized Node preload environment,
checked stderr/exit/signal policy and synchronous child kill/reaping on timeout.
Build invocations have a separate 30-second timeout after compiling the driver.
The runner removes managed copies on every failure. Trusted fixture code must
not launch detached descendants; this is not a sandbox/process-tree jail.
The regression suite checks actual emitted mismatch/stderr/nonzero/timeout
failures, output bounds, cleanup, unsupported reporting and child reaping.
Node 22/current repository Rust CI builds/runs this unified command.
Runtime/interop/platform limits are in
[`bundler/README.md`](../../crates/bundler/README.md).
