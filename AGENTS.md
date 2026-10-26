# Making Changes

Before commiting any changes, check:

- have I written necessary tests for any code changes?
- have I run `./scripts/spot-check`?
- have I updated any out of date README.md files in the crates I have changed?

## Notes

- `cargo` commands can take a long time to compile. If you read `Compiling <foo>` in the output, DO NOT kill the command, wait for it.
- Bazel shell checks that invoke `cargo` should include `@rules_rust//rust/toolchain:current_cargo_files` and prepend the toolchain `bin` directory from runfiles to `PATH`.

## Style

- Integration tests that require real filesystem access should be run from within `tempdir` managed temporary directories
- never use `#[allow(lint)]`, always use `#[expect(lint)]` in case the underlying issue is ever fixed.
  If it is only errors in some build configurations, use e.g. `#[cfg_attr(feature = 'test', dead_code)]`
- Always use `eyre` for errors, never use `thiserror`
- Add lazy `WrapErr` context at pipeline/IO boundaries, retaining typed causes.
  Include action and relevant paths/requests in errors, never in tracing fields.
  Error-chain assertions use alternate report formatting, not just `to_string()`.
- Use `insta` for general purpose snapshot testing
- Use `inquire` for interactive user prompts
- Use `cargo bench` for benchmarking. Never use `criterion`.
- Raw git operations are usually kept in a `crate::git` module
- Prefer using `git2` for git operations over calling the `git` CLI, unless we are using git network operations.
- Use `prompt-manager` for LLM prompts, not builtin strings.

## Organization

Persisted cache trees are read through borrowed accessors, including nested
syntax/emission/execution records and Ecco precursors. Keep storage fields
private in record modules; scoped construction APIs are for extraction before
provenance sealing, not consumer mutation. Owned adapter moves are explicit.
Scalar IDs/spans and enum discriminants/payload matching remain value APIs.
Production source sidecars are typed FlatBuffers, schema 19/extractor 29.
Keep structural/integrity verification and borrowed semantic validation before
constructing checked cache records; never deserialize/unpack trees on hits.
Serde projections remain for declaration fingerprints and debug/test fixtures.

Leave CONCISE notes on persistent design in AGENTS.md files in the roots of the crates, or the repo root for cross-cutting concerns.
Any temporary notes about mid-refactor or mid-implementation belong in the NOTES file instead.

- TODO.md files are formatted as a series of sections
- Implementation notes go in NOTES.\*.md for the current workplan.
- Architectural notes go in AGENTS.md. no long-form guides or source code excerpts
- `-cli` crates export both a lib and a bin. The lib exports the clap definition so commands can be embedded as subcommands in other binaries (e.g., `mx`)

## Bundler fixtures

- `fixtures/bundler/` holds portable runtime contracts, not generated-code snapshots.
  Preserve pinned upstream provenance, adaptation notes, and all applicable licenses.
- `cargo build -p bundler-cli && node scripts/bundler-fixtures.mjs`
  validates all corpus metadata, then source observations and public build/output
  in one run. Report input/build/backend/output failures separately, continue
  later cases, and never build invalid inputs or count legacy passes as native.
- Bundler target: scope-hoisted direct ESM bindings with native imports/exports
  only between actual output chunks; root sets describe reachability, not a
  mandatory physical partition. Public entry/export boundaries constrain output.
  Each CJS input is one opaque segment plus a logical ESM facade, both subject
  to normal usage/link/grouping. The registration/cell/facade-family backend
  is deleted; linked emission is native-only, including low-level roots.
  Preserve activation timing; no source-loading runtimes or runtime compilation.
- All public and low-level roots use native ESM/CJS chunks. CJS availability
  chunks contain pure lexical cached initializers; only demanded facade jobs
  execute factories/snapshot properties. Multi-owner roots are evaluation-only
  native entry wrappers; they never invent a combined public namespace.
- `fixtures/bundler-apps/` is the separate real-browser corpus. Use agent-browser
  to compare pinned source and native public output; preserve source failures,
  browser diagnostics, exact upstream bytes and all licenses.
