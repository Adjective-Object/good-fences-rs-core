# Portable bundler semantic fixtures

These small upstream adaptations and locally authored cases describe observable
JavaScript runtime semantics. **The baseline validates the fixtures using Node; it
does not run or establish conformance of any bundler.** There is no bundler
implementation behind this command.

From the repository root, with Node.js 18 or newer:

```sh
node scripts/bundler-fixtures.mjs
```

No package installation, build, upstream engine, or network access is needed.
An optional positional argument selects a different corpus root. The runner
fails for missing/empty corpora, malformed or unsupported metadata, execution
errors, timeouts, stderr, or output differences; it never silently skips cases.

The corpus contains six esbuild cases and six rspack cases covering
shared entrypoint identity, static/dynamic imports, CJS default/named interop
and cycles, ESM live bindings, re-export cycles, and evaluation order.
Two authored cases add native-required lexical TDZ and mixed successful,
throwing and rejecting entrypoints with preceding observable effects.
Rspack-derived webpack fixtures retain webpack's notices as well as rspack's.
The dedicated Node 22 CI job runs both this corpus and the runner's own tests.

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

The only accepted `backend` requirement is `"native"`. The baseline validates
that field but makes **no bundler-backend claim**. Differential mode checks the
actual production selection and fails if such a case falls back to legacy.
Unmarked cases may currently pass on legacy; that is not native-linker coverage.

Sources must be self-contained JavaScript and local data. No external packages,
`node_modules`, project sources, downloads, Jest, webpack/rspack globals, or
package lifecycle scripts are supported. The runner never installs dependencies
or invokes scripts; missing runtime dependencies fail execution. Node built-ins
are available. Only trusted, reviewed fixtures should be run: Node execution is
**not a sandbox**, and this runner does not enforce a network or filesystem jail.

## Execution and differential adapter

Cases are discovered in deterministic ID order and all manifests are validated
before execution. Each case's `files/` tree is copied into a fresh managed
directory under project-local `target/p1-test-tmp` by default. Each
entrypoint runs in a separate Node process with that directory as its cwd,
in manifest order. Entrypoints share the case's copied filesystem, not process
state. The runner removes the copy in `finally` on success and failure.
Node preload/search overrides are removed and coverage is disabled. Each entrypoint
has a five-second timeout and a 1 MiB output-buffer limit.

For both successful and expected-failure entries, one shared ESM observation
driver imports the absolute entry path through Node's normal file loader.
Completion (null or normalized error) travels over a separate pipe, never stdout
or stacks. Premature exit, including exit zero, cannot count as completion.
This observes imported module execution, not CLI-only `require.main` semantics.
It uses no preload or custom loader hooks. Fixture code remains trusted, not jailed.

Bundler adapters must separately bundle these sources and execute the resulting
entrypoints, comparing the same per-entry stdout and normalized error observations.
They must explicitly declare
unsupported capabilities, not count skipped fixtures as passes. Linking,
chunking, diagnostics, incremental invalidation, and bundler-only behavior need
their own drivers and expectations. Chunk names, chunk counts, generated-code
snapshots, and incidental formatting are not this corpus's semantic contracts.

Runner regression tests use synthetic fixtures only:
`node --test scripts/bundler-fixtures.test.mjs`.

## Public-pipeline differential mode (native Node 22)

The original baseline command above is unchanged. A separate harness now builds
through the public Rust library and transactional publisher:

```sh
cargo build -p bundler --example build
node scripts/bundler-differential.mjs
node --test scripts/bundler-differential.test.mjs
```

It validates the same manifests/provenance first, makes **separate** source and
bundler copies, and compares manifest expectations with source and emitted
per-entry stdout and normalized runtime outcomes through the shared observer.
All fourteen contracts/nineteen emitted entry executions pass, with zero
unsupported skips. The actual backend is reported for every built case:

- **Native (4):** `authored/native-tdz`, `esbuild/unused-import-side-effects`,
  `rspack/esm-effect-order`, `rspack/esm-live-binding`.
- **Legacy (10):** all other cases, including `authored/runtime-outcomes` and
  `esbuild/dynamic-import-tla-order`. These passes are not native-backend coverage.

These observations do not establish general bundler conformance. It additionally
executes an authored same-runtime concurrent multi-entry CJS/ESM identity case
and four bundler-only source-range diagnostic cases (nonliteral import,
unconfigured external, require-of-async-ESM, TS emission).
No upstream files, provenance or licenses are changed.

`runDifferential(root, {driver, tempRoot, timeoutMs, unsupported})` is the small
programmatic harness API. `unsupported` must explicitly map known case IDs to
nonempty reasons; results say `unsupported`, with zero emitted executions, never
`passed`. Unexpected build failures remain failures, not inferred skips.
Passed results contain `{id, status: "passed", backend: "native" | "legacy",
executions}`. The baseline API `runFixtures(root, {timeoutMs, tempRoot})` keeps
returning `{id, stdout}` and never reports a bundler backend. The shared
`observeEntries` helper returns `{stdout, outcomes}`, where each ordered outcome
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
Dedicated Node 22/current repository Rust CI builds/runs this separate mode;
native baseline CI remains Rust-free. Runtime/interop/platform limits are in
[`bundler/README.md`](../../crates/bundler/README.md).
