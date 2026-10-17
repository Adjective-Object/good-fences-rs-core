# Incremental bundler implementation handoff

## Current scope

## P3.1 checkpoint

- `source_graph::from_owned` consumes existing `ModuleFacts` and segments, with
  per-`DependencyId` resolution inputs, never a second parse IR. Typed edges
  distinguish binding/evaluation/execution, local references, activation,
  source site, read/write/type flags and enclosing-function metadata.
- Module nodes retain empty targets. Forward/reverse adjacency indexes the
  identical edge records. Duplicate specifiers and dynamic sites stay distinct;
  deferred CJS require stays synchronous with function containment recorded.
- Verification: 23 source_graph tests (two new) and `./scripts/spot-check`
  pass using repository-local TMPDIR. TagGraph/report migration is P3.4;
  missing/external resolution diagnostics and revision APIs are P3.3.

Phase 0 is historical planning/fixture work. Current scope is autonomous P1
onward in narrow linear commits. P0–P2 are implemented; P3–P9 remain pending.
The bounded P2 ownership ends here, before P3.

## P2.3 checkpoint — source-local independence and invalidation

- Cold, warm and cache-disabled records match `parse_module` facts, body and
  ordered statement ranges exactly, including ESM aliases/star/default exports,
  destructuring, lexical shadows/forward references, TS declarations/import
  types, Unicode, directives/hashbang and opaque CJS wrapper/deferred sites.
- Same-length edits with explicitly restored identical mtime invalidate by
  source hash. JS/TS/JSX/definition changes invalidate copied records. Effective
  parser modes match pinned parsing for all 11 supported extension forms and
  both package formats; package `type` changes reparse `.js`, while explicit
  ESM/CJS extensions override package policy and stay hits when appropriate.
- Relocated identical source/sidecars remain hits and re-resolve raw specifiers
  through real `MonorepoResolver` instances under distinct roots. Neither root
  appears in persisted bytes. Real resolver extension choices select different
  targets without changing source facts; package exports/sideEffects, tsconfig
  and external effects-policy edits leave cache bytes and mtime unchanged.
  No P4 policy evaluator or P3 graph behavior is claimed.
- Deleted source reads fail despite orphan caches; deleted dependencies fail
  fresh resolution while the importing source remains a hit. Identical source
  recreation may legitimately reuse its orphan cache. Diagnostics are never
  persisted: current source/path/policy regenerates errors, including generated
  markers, relocated ESM-as-CJS failure and non-UTF-8 source. Prior good sidecars
  survive errors but cannot provide stale successful results.
- Added a workspace OXC revision guard and kept decoder dependency-link checks
  linear using a source-local lookup map (not persisted). No resolver crate or
  legacy tracker behavior was modified. READMEs/TODO/PLAN document the public
  projection and the end of P2; P3 remains pending.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache
  -p ast-segmenter -p oxc_utils_parse -p import_resolver -p source_graph
  -p tag_graph -p unused_finder -p good_fences` passes **267 tests**:
  cache 25, parser/extractor 47+9, resolver 18, graph 21+17, consumers 70+60.
  Both existing owned-DTO insta snapshots pass unchanged.
  `cargo clippy -p segment_cache --all-targets --no-deps` passes with
  only the existing ftree_cache lifetime warning. `./scripts/spot-check` passes
  before every P2 milestone commit (47 parser tests and workspace compilation).
  Linux runtime validation only; Windows replacement behavior was inspected in
  pinned tempfile, not executed. Full-workspace tests/formatting are not claimed.

## P2.2 checkpoint — atomic source-adjacent sidecars

- `load_or_parse` reads exact source bytes, validates cache or parses, and
  returns source/record plus explicit hit/miss and write outcomes. Cache IO
  problems are nonfatal; source/parser/semantic/support errors are fatal.
  `CacheMode` provides read-write, read-only and disabled behavior. Negative
  diagnostics are not stored and old successes never bypass parse failures.
- Unique sibling staging uses existing locked tempfile 3.27.0. Its pinned
  implementation was inspected: overwrite uses Unix rename and Windows
  MoveFileExW with MOVEFILE_REPLACE_EXISTING, not delete-before-replace.
  Flush/sync precede publication; errors and unwinds clean staging files.
  Crash staging orphans are ignored; no writer sweeps another writer's files.
  This promises atomic visibility, not directory-fsync power-loss durability.
- Independent publishers are last-writer-wins. Re-read source immediately
  before publication and before returning both cold/warm results. Detected
  edits/deletion cannot publish/return stale snapshots; no cross-file lock
  transaction is claimed against noncooperating editors after the last check.
- Source files are untouched. Read-only sources work with writable parents;
  permission/replacement errors preserve previous records and return uncached
  facts plus an explicit write error. Requested lexical paths own sidecars;
  symlink source aliases have independent caches and symlink sidecar targets
  are replaced, not overwritten. Watcher artifact exclusion helper is public.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache`
  passes 15 tests. Added managed-directory coverage includes 64 concurrent
  publications with reads, interruption/error/unwind cleanup, write failures,
  source mutation/deletion races, stale parse success, cache modes, corruption/
  truncation/oversize repairs, permissions and symlink behavior.
  `./scripts/spot-check` passes (47 parser tests and full workspace check).
  Windows semantics are pinned-API inspected, not runtime-tested on this Linux host.

## P2.1 checkpoint — deterministic bounded codec

- New `segment_cache` crate reuses `ModuleFacts` and `ModuleBody`, with statement
  ranges as the source-local segment projection. It intentionally does not
  serialize legacy usage tags/name sets or create a parallel occurrence IR.
- CBOR schema/extractor versions and exact pinned OXC revision accompany source
  SHA-256/length and effective JS/TS/JSX/definition/CJS config. Parser defaults
  and checked extraction policy belong to the extractor version. A checksummed
  envelope, canonical re-encoding and byte/item/depth preflight reject damaged,
  ambiguous, oversized or incompatible data before trusting facts.
- Validate every UTF-8 range, IDs, binding/dependency links, occurrence ordering,
  and the one-body CJS invariant. Owned facts remain available after arena drop.
  serde_cbor 0.11.2 uses existing serde 1.0.218 without raising its constraint;
  SHA-256 0.10.9 was already locked. READMEs preceded source/package edits.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p segment_cache`
  passes four tests, including five deterministic roundtrip modes, every-byte
  corruption/truncation checks and malformed length/depth/allocation inputs.
  `./scripts/spot-check` passes (47 ast-segmenter tests and workspace check).
  Only touched Rust files are rustfmt-formatted; workspace-wide fmt check
  exposes unrelated pre-existing formatting differences.

## P1.5 checkpoint — opaque CommonJS and explicit support gate

- `ParsedModule::body` / `ModuleBody::CommonJs { span }` identifies exactly
  one executable segment, spanning the complete body except hashbang, including
  directives, comments, whitespace and top-level returns. Even an empty CJS
  module has one opaque segment. No inferred CJS named export list or
  independently removable internal segments are returned.
- `Reference::commonjs` classifies unresolved require/module/exports wrapper
  occurrences independently of lexical shadows. `Dependency::enclosing_function`
  records lexical containment, not eager execution permission; after-return,
  repeated and nested dependencies remain separate. `DependencyKind::ImportType`
  retains TS type import sites without turning them into execution edges.
- Preserve `module.exports`, `exports` alias and module-variable reassignment
  verbatim. Reject require reassignment/aliases/indirect loaders, unsupported
  module member/escape accesses, wrapper-scope redeclarations and wrapper
  arguments escapes rather than guessing missing dependencies. Nested lexical
  and var shadows and type-space-only wrapper names have regression tests.
- `policy` rejects mixed ESM/CJS, nonliteral/unsupported loader shapes,
  unshadowed eval, nested with, import.meta, TLA/for-await, import attributes/
  options/phases, JSX and TS/decorator/auto-accessor/resource-management runtime
  transforms with path/range diagnostics. Erasable TS remains parse-only;
  ambient and merged type/value bindings and implicit type-only local/default
  exports retain accurate metadata.
- The ast-segmenter README support matrix explicitly leaves target-dependent
  asset/native-addon/external/require-of-ESM checks to later resolution/linking.
  Those raw specifiers are retained and tested, never silently omitted or
  claimed executable. Low-level parser and legacy adapters retain their broader
  policies. No P2 or later implementation was started.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p ast-segmenter
  -p oxc_utils_parse -p source_graph -p tag_graph -p unused_finder
  -p good_fences` passes 224 tests (47/9/21/17/70/60), including both insta
  snapshots and UTF-8 declaration/reference/dependency spans.
  `TMPDIR="$PWD/target/p1-test-tmp" ./scripts/spot-check` passes, including
  workspace compilation. Existing unrelated lifetime/dead-code warnings remain.

## P1.4 checkpoint — owned lexical and dependency occurrences

- Added `ast_segmenter::owned` DTOs via `ParsedModule::facts`, leaving
  `Segment`, `VariableScope`, `RawModuleDeps` and their legacy consumers intact.
  Binding IDs identify first declaration byte positions; dependency IDs
  identify expression/declaration byte positions within the exact source.
  Resolved lexical references do not depend on statement order. No OXC IDs
  or borrowed AST data escape.
- Preserve all binding declarations (including var redeclarations), references
  with read/write/type-only metadata, imported local aliases, local/default/
  string-named/star/namespace exports, repeated static/dynamic/require sites,
  directives and hashbang. Anonymous default expressions are not falsely
  identified as live local bindings. Nonliteral dependency calls fail.
- Tests cover lexical/var forward uses, nested closures, scope shadowing,
  object/array/default/rest destructuring, catch and method parameters,
  repeated imports/requires, type/runtime references and arena reset/reuse/
  drop. An insta snapshot pins deterministic owned DTO output.
- Verification: `TMPDIR="$PWD/target/p1-test-tmp" cargo test -p ast-segmenter
  -p source_graph -p tag_graph -p unused_finder -p good_fences
  -p oxc_utils_parse` passes 216 tests (39/21/17/70/60/9).
  `TMPDIR="$PWD/target/p1-test-tmp" ./scripts/spot-check` passes, including
  workspace compilation. Existing lifetime/dead-code warnings remain.
  CJS opaque extraction and the executable support matrix remain P1.5.

## P1.3 checkpoint — explicit parser policy and checked extraction

- `oxc_utils_parse::parse_bundler_file` uses extension-sensitive JS/JSX/TS/TSX
  syntax, an explicit package `ModuleFormat`, and explicit-extension overrides.
  Unsupported extensions and parser errors/aborts return eyre diagnostics with
  path and OXC diagnostic ranges. No package discovery or file IO is implied.
- `ast_segmenter::parse_module` validates semantic early errors before checked
  extraction and returns only owned `ParsedModule` records. A duplicate lexical
  declaration is accepted by the low-level parser but rejected at this gate.
  `try_segment_file` rejects unsupported statements and guards against partial
  statement output; legacy `segment_file` remains available for tracker callers.
- Generated markers do not skip bundler input, even when the source is invalid.
  Tracker compatibility still accepts TS in JS/unknown extensions, uses content
  module/script detection, and preserves its generated-file skip rule.
- Low-level parsing supports CommonJS syntax including top-level return.
  Bundler extraction explicitly rejects CommonJS pending P1.5's opaque body;
  TS import/export assignments and namespace exports also fail explicitly.
  Accepted TS/JSX records are not transformed/emitted JavaScript. Binding
  occurrences, directives/hashbang records and the broader support matrix
  remain P1.4/P1.5 work.
- Added eyre at workspace level for the new fallible APIs. Parser README existed
  before package edits; crate documentation records the separate policies.
- Verification: all 211 tests across oxc_utils_parse, ast-segmenter,
  source_graph, tag_graph, unused_finder and good_fences pass with managed
  temporary directories; `./scripts/spot-check` passes. Existing
  lifetime/dead-code warnings remain.

## P1.2 checkpoint — spans and resolved report dependencies

- Parser and public-report assertions pin zero-based, end-exclusive UTF-8 byte
  ranges, including offset zero, multibyte source, unused files/symbols/segments,
  and tagged type exports. Fixed the report test helper's stale `start - 1`
  slicing; production offsets are unchanged.
- Reproduced relative dependency reporting missing the first target segment:
  expected unused `[1]`, actual `[0, 1]`. Preserve the existing resolver mapping
  through `ResultFile` rather than looking raw specifiers up as absolute paths.
  Filesystem tests cover static/dynamic imports, re-exports, execution-only
  imports and require; clearing downstream symbol tags isolates segment edges
  from upstream tag propagation.
- Reproduced execution imports reaching no statements in no-export targets.
  Index each file's contiguous node range so execution-only imports and opaque
  requires reach all statements without scanning the whole graph per edge.
  Tests cover targets with and without exports. This is conservative usage
  analysis, not eager bundler evaluation of deferred require bodies.
- Verification: all 136 tests across ast-segmenter, source_graph, tag_graph and
  unused_finder and `./scripts/spot-check` pass. Filesystem tests use managed directories under
  `target/p1-test-tmp`. Existing lifetime/dead-code warnings remain.

## P1.1 checkpoint — type-only compatibility

- Reproduced with `cargo test -p unused_finder test_typeonly_interface_allowed`:
  `test test::test_typeonly_interface_allowed ... FAILED`, expected `[TypeOnly]`
  but actual `[Entry, TypeOnly]` on `MyInterface`.
- Root cause: extraction collapsed OXC type/value references into an untyped
  escaped-name set. Both tag traversals treated generic constraints as runtime
  intra-file dependencies. Retain a per-name type-only flag (AND across uses),
  and honor `follow_type_only` in forward and reverse traversal. No report
  filtering, changed expectations, or blanket suppression of type root tags.
- Added extraction coverage for constraints, annotations, implements, typeof,
  class inheritance, unresolved names, and mixed type/runtime references in
  both orders. Graph tests cover both traversal directions, both policies,
  entry/test/ignored tags, and explicit roots. Full-report filesystem tests
  cover ordinary/type imports and runtime re-exports; existing assertions stay.
- Verification: focused extraction and graph regressions pass; all four
  `cargo test -p unused_finder typeonly` tests pass. `cargo test -p ast-segmenter
  -p tag_graph -p unused_finder -p source_graph` passes 132 tests (27/17/67/21).
  `./scripts/spot-check` passes. Existing lifetime/dead-code warnings remain.
  Tests used managed directories under repository-local `target/p1-test-tmp`
  via `TMPDIR`, not system temporary paths.
- Adjacent probe, outside P1.1: a static import through a named re-export
  barrel reaches the leaf but leaves the barrel unused. With main importing
  `Consumer` from barrel and barrel re-exporting it from lib, the report diff was
  `unused_files: ["<root>/packages/root/barrel.js"]` and
  `unused_symbols: {"<root>/packages/root/barrel.js": [SymbolReport {
  id: "Consumer", start: 17, end: 50 }]}` instead of empty collections.
  Static-import resolution currently skips intermediate re-export segments.
  This separate linking/report issue is not normalized into a passing expected
  report or fixed here; the committed regression tests direct imports and
  re-exports. Revisit with P3 linking/shared-edge work.

## Completed phase-0 integration (historical)

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

## Phase-0 parallel ownership (historical)

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
