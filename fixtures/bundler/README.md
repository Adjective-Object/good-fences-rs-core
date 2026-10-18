# Portable bundler semantic fixtures

These small, attributed upstream adaptations describe observable JavaScript
runtime semantics. **Phase 0 validates the fixtures themselves using Node; it
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

The initial corpus contains six esbuild cases and six rspack cases covering
shared entrypoint identity, static/dynamic imports, CJS default/named interop
and cycles, ESM live bindings, re-export cycles, and evaluation order.
Rspack-derived webpack fixtures retain webpack's notices as well as rspack's.
The dedicated Node 22 CI job runs both this corpus and the runner's own tests.

## Layout and manifest

```text
<upstream>/LICENSE
<upstream>/<case>/case.json
<upstream>/<case>/files/entry.js
```

Every case uses this complete schema (no additional fields):

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

Sources must be self-contained JavaScript and local data. No external packages,
`node_modules`, project sources, downloads, Jest, webpack/rspack globals, or
package lifecycle scripts are supported. The runner never installs dependencies
or invokes scripts; missing runtime dependencies fail execution. Node built-ins
are available. Only trusted, reviewed fixtures should be run: Node execution is
**not a sandbox**, and this runner does not enforce a network or filesystem jail.

## Execution and differential adapter

Cases are discovered in deterministic ID order and all manifests are validated
before execution. Each case's `files/` tree is copied into a fresh managed
temporary directory (honoring the OS temporary-directory environment). Each
entrypoint runs in a separate Node process with that directory as its cwd,
in manifest order. Entrypoints share the case's copied filesystem, not process
state. The runner removes the copy in `finally` on success and failure.
Node preload/search/coverage environment overrides are removed. Each entrypoint
has a five-second timeout and a 1 MiB output-buffer limit; nonzero exits, signals,
stderr, and output mismatches fail the case.

Bundler adapters must separately bundle these sources and execute the resulting
entrypoints, comparing the same observable output. They must explicitly declare
unsupported capabilities, not count skipped fixtures as passes. Linking,
chunking, diagnostics, incremental invalidation, and bundler-only behavior need
their own drivers and expectations. Chunk names, chunk counts, generated-code
snapshots, and incidental formatting are not this corpus's semantic contracts.

Runner regression tests use synthetic fixtures only:
`node --test scripts/bundler-fixtures.test.mjs`.

## Public-pipeline differential mode (Node 22.15+)

The original baseline command above is unchanged. A separate harness now builds
through the public Rust library and transactional publisher:

```sh
cargo build -p bundler --example build
node scripts/bundler-differential.mjs
node --test scripts/bundler-differential.test.mjs
```

It validates the same manifests/provenance first, makes **separate** native and
bundler source copies, and compares original manifest stdout, native stdout and
emitted launcher stdout for every entry. All twelve portable cases/fifteen
emitted entry executions pass, with zero unsupported skips. It additionally
executes an authored same-runtime concurrent multi-entry CJS/ESM identity case
and four bundler-only source-range diagnostic cases (nonliteral import,
unconfigured external, require-of-async-ESM, TS emission).
No upstream files, provenance or licenses are changed.

`runDifferential(root, {driver, tempRoot, timeoutMs, unsupported})` is the small
programmatic harness API. `unsupported` must explicitly map known case IDs to
nonempty reasons; results say `unsupported`, with zero emitted executions, never
`passed`. Unexpected build failures remain failures, not inferred skips.
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
