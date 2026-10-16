# Incremental bundler implementation handoff

## Current scope

Ground the phased design in the current OXC/segment graph implementation and
upstream bundlers. Phase 0 imports a small, portable runtime fixture corpus from
esbuild and rspack; it does not claim to implement a bundler.

## Completed integration

- esbuild is checked out at `../esbuild`, pinned for this research to
  `f6058f8364fe7ab91ca57a83e02577ed74c9cae4`.
- Imported six esbuild and six rspack cases, with pinned provenance, adaptation
  notes, and upstream MIT notices including webpack's inherited notice.
- `node scripts/bundler-fixtures.mjs` passes all 12 baseline contracts
  (15 entrypoint executions); no bundler was invoked.
- `node --test scripts/bundler-fixtures.test.mjs` passes all 31 runner tests.
- `./scripts/spot-check` passes; this does not establish full-workspace test
  parity or resolve the pre-existing failures in `TODO.followups.md`.
- `.github/workflows/unittest.yml` runs the runner tests and corpus baseline
  in a separate Node 22 job without installing project dependencies.
- The scoped `.gitignore` exception preserves fixture `index.js` files.
  No fixture files remain ignored.
- `PLAN.incremental-bundler.md` and `TODO.incremental-bundler.md` describe the
  remaining increments. Source-local IR, disk caching, executable linking,
  effect analysis, chunk assignment, and bundle emission are not implemented
  by this increment.

## Parallel ownership during this increment

- Architecture: `PLAN.incremental-bundler.md` and
  `TODO.incremental-bundler.md`; inspect current crates without modifying them.
- esbuild research/import: `fixtures/bundler/esbuild/` and
  `NOTES.incremental-bundler.esbuild.md`.
- rspack research/import: `fixtures/bundler/rspack/` and
  `NOTES.incremental-bundler.rspack.md`.
- Integration: fixture runner, shared documentation, validation, and commits.

## Phase 0 fixture interchange

Each upstream directory contains its upstream `LICENSE` and case directories.
Each case has `case.json` and a `files/` directory containing standalone JS
sources. Use this manifest shape:

```json
{
  "id": "upstream/descriptive-case",
  "upstream": {
    "repository": "https://github.com/owner/repo",
    "commit": "full commit SHA",
    "paths": ["path/to/original/test"],
    "test": "original test name",
    "license": "MIT",
    "adaptations": ["Describe every semantic or harness adaptation."]
  },
  "moduleType": "module",
  "entryPoints": ["entry.js"],
  "expectations": {
    "stdout": ["deterministic output line"]
  },
  "goals": ["Semantic properties exercised by the case."]
}
```

`moduleType` is `module` or `commonjs`. Entrypoints are relative to `files/`,
executed in manifest order in separate Node processes; expected stdout is the
concatenation of their output lines. Copy any required package metadata into
`files/`; the runner supplies a root package.json only when absent.
Cases must not require external packages, downloads, Jest, webpack/rspack
globals, an upstream bundler, or project sources. Replace upstream assertion
harnesses with deterministic runtime observations and document the adaptation.
Avoid adding output-format or chunk-count assertions: emitted-code shape is
not a portable semantic contract. Preserve license notices and record exact
provenance; do not copy fixtures with unclear licensing.

Baseline execution checks the fixtures, not bundler conformance. Linking,
chunking, diagnostics, and bundler-only cases will need explicit future test
drivers rather than being counted as passing by this runner.
