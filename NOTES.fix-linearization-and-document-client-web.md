# Linearization fixes and client-web exercise

## Scope and checkpoints

- Started 2026-10-21 on `fix-linearization-client-web`, based on
  `integration/segment-aware-linear` at `e18238c`.
- Original dirty `segment-aware` and `perf-optim` worktrees remain untouched.
- Removed inactive clean worktrees with `wt remove --no-delete-branch --no-hooks
  --foreground`, preserving all branch checkpoints: `resolved/perf-optim`,
  `resolved/bundler-browser-compat`, `resolved/segment-relocation`,
  `bundler-browser-compat`, and `integration/segment-aware-linear`.
  Windows refused deletion of two now-empty directories after unregistering
  those worktrees; no force/reap or process termination was used.
- No client-web source, generated source, traces or proprietary inputs will be
  committed or uploaded. Record commands, aggregate measurements and findings
  here; keep input-dependent artifacts local.

## Windows publication failures

Both reported failures reproduced on the new branch before changes.

- `publish::tests::cancellation_failure_and_busy_writer_leave_previous_outputs_intact`:
  backtrace points to `snapshot` reading `.bundler-lock` while the test itself
  holds an exclusive lock. Exclude lock coordination bytes from output snapshots;
  retain checks of publication metadata, generations, file contents and mtimes.
- `sidecar::tests::concurrent_publishers_always_leave_one_complete_record`:
  concurrent `tempfile::persist` calls fail with OS error 5. The pinned tempfile
  uses `MoveFileExW` alone. The pinned Rust standard library additionally handles
  access-denied replacement through handle-based POSIX rename semantics.
  Use `std::fs::rename` with a closed, RAII-managed sibling staging path; retain
  flush/sync, source recheck, atomic replacement and error cleanup.

The concurrent regression now synchronizes all eight writers and retains an
open reader of the original sidecar through all 64 replacements, then checks
its original complete bytes. All eight publication tests pass with no exclusions;
the cache concurrency regression also passes after the change.

## Per-file FlatBuffers completion

Recovered omitted commit `77f924a` (AST borrowed views) and the dirty per-file
storage patch from `perf-optim`; original worktree remains unchanged.
Resolved replay conflicts in favor of the integrated connection planner and
activation ordering, adapting it to value views rather than restoring the old
planner. The production sidecar path is now `.seg.fb`.

The skipped wire schema lacked integrated dynamic namespace observations,
`uses_this`, deferred emission alternatives and async-reference evidence.
Added these to the authoritative schema and regenerated with pinned flatc
25.2.10. Extractor 16 preserves the integrated parser policy and rejects older
records. Views retain borrowed strings, tables, optional property vectors and
initializer slots; no unpack-on-hit bridge.

Windows fixes are checkpointed in `e08d0f2`. FlatBuffers integration passes all
728 affected library tests: AST 75, Ecco 112, segment cache 46, source graph 38,
wire schema 11 and bundler 446, with no exclusions. The required
`scripts/spot-check` also passes, including the full workspace check.

The first run used the machine's Node 22.8 and failed ten runtime tests;
rerunning the bundler suite with the retained Node 22.14 executable passes all
446. This includes the TLA rejection-order case, so that discrepancy was not
reproduced under Node 22.14. The retained Node 24.10 executable instead fails
256 tests, predominantly with `EISDIR: lstat 'C:'` before executing the scripts.
Use Node 22.14 for this exercise; do not weaken tests to accommodate either
runtime's failures. A synthetic wire fixture also needed the newly required
deferred union; it now supplies the explicit unsupported alternative.

After the requested pause/restart, `cargo` resolves to
`C:\Users\mahuangh\AppData\Local\mbx\bin\cargo.exe`. Builds use that transparent
cache wrapper without `CARGO_TARGET_DIR` or `RUSTC_WRAPPER` overrides.

## Client-web exercise

Publication fixes and FlatBuffers integration are complete. Start with
isolated small packages, increase dependency graph size, then attempt the full
mail workload with all application and worker entries. Distinguish preparation,
resolution, build, output and runtime failures; never count a successful
transpilation or build as runtime correctness.

### Small-package checkpoint

Release example built through `mbx`; the wrapper also reported nine cache hits
on the all-targets check. All 65 portable runtime fixtures pass with the release
driver and native backend. The fixture invocation initially returned failure
because the local wrapper expected status `pass`, whereas the harness returns
`passed`; inspection of all 65 actual result records confirmed no fixture failure.

Local client harness runs three fresh processes per case, validates native backend,
syntax-checks every emitted executable, compares output filenames/content hashes,
and requires zero source parses and zero artifact writes on the second/third runs.
Initial timings below are observations, not benchmarks: portable validation ran
concurrently, and later preparation introduced additional filesystem contention.
They are not a pre/post-migration speedup comparison.

| Input | Script/resource cache hits on warm run | Initial / two warm builds | Observations |
| --- | ---: | --- | --- |
| debounce | 2 | 2.21 s / 70, 75 ms | One export; cancellation/latest-arguments behavior matches source |
| error codes | 29 | 328 ms / 167, 171 ms | 29 exports; exported data and validation/accumulation behavior match source on corrected harness rerun |
| lodash browser adaptation | 641 | 9.08 s / 6.83, 6.86 s | 322 exports; nine value/collection/template/chaining cases match source |
| service-worker common | 7 | 10.56 s / 10.70, 8.06 s | Build, syntax and repeated-byte checks only; no worker runtime claim |
| expansion animation | 4 | 7.61 s / 7.00, 6.84 s | Build, Sass/CSS output and repeated-byte checks only; no browser runtime claim |

The source comparison host needs a local loader for extensionless relative
imports in transpiled TypeScript. It probes `.js`/`index.js` only inside staged
source, never generated bundler output. Independent module namespaces/functions
cannot be compared by identity; compare exported data and observed function
results instead. Both harness issues were corrected rather than reported as
bundler failures.

Preparation uses installed tsgo with ES2020/ESNext, React JSX and legacy
decorators, `noCheck`/`noResolve`; this is transpilation, not typechecking.
Sass compiles using installed dependencies and preserves `.scss` import spelling
for `ext:.scss`. Resolver conditions are `browser`, `fluentIconFont`, `style`,
not `source` for the compiled tree. Staged workspace manifests use ESM package
mode. The existing isolated lodash browser corpus disables the three vendor
CommonJS-detection probes; these observations do not cover an unadapted Node
lodash build. No original client source/manifests were changed.

### Performance observations

A warm service-worker trace in the larger prepared root took 14.16 s.
`build.recheck` alone took 6.25 s; preparation took 11 ms, aggregate seven-file
cache decoding 4.2 ms, linking 7 ms and publication 4 ms. Initial inventory is
outside a dedicated span; the remaining multi-second gap and code path point
to filesystem inventory as well, but that part is inference, not an independently
measured stage. The inventory recursively canonicalizes/stat-checks input-tree
entries, then repeats the work during final recheck. Seven reachable files in a
much larger staged root therefore cost seconds despite complete FlatBuffers
hits. Preserve correctness rechecks; a reachable-input/directory-invalidation
design needs separate work rather than skipping them.

### Full application preparation

Prepared the complete dependency closure in a separate session-local root,
using the actual `OWA Mail` workflow: mail bootstrap, data worker, analytics
worker, and the vendor editor index-readjuster worker. Unlike the earlier
45-package probe corpus, dependencies of application roots are included.
Preparation reports missing packages, version collisions, transpilation and
Sass failures separately.

- Enumerated 4,710 package names and 70,135 TypeScript inputs. Installed
  TypeScript correctly classifies 4,973 arbitrary-extension files such as
  `*.d.json.ts` and `*.d.scss.ts` as declaration-only: no JavaScript emission is
  expected. There are 65,162 executable TS inputs. All 71 native compiler
  batches exited successfully; this still is not typechecking.
- A monolithic native TypeScript emit consumed roughly 13-15 GiB resident
  memory, leaving 1.6 GiB free on the 31.7 GiB host. Stopped only that source
  preparation process, not Cargo. Retried with at most 1,000 files per process,
  `GOMAXPROCS=2` and `GOMEMLIMIT=2GiB`; batched compiler time was 629.6 s.
  Initial copying alone took 993.5 s. These are preparation costs, not bundler
  timings or cache-hit costs.
- The initial flat staging reported 613 nested-version conflicts. Reconstructed
  installed vendor topology, visiting 2,568 installed vendor locations and
  copying 862 missing/corrected locations. Audited all 2,568 manifests against
  the installed originals; there are no semantic/version mismatches, and
  restored exact original manifest bytes where initial formatting differed.
  Repair took 403.4 s and overlapped other preparation.
- Compiled all 2,474 Sass inputs. The first pass took 252.1 s and left 38 importer
  errors. Retried only those failures after correcting the local importer's
  `src/` versus `lib/` mapping and retaining vendor `style`/Sass export handling;
  all now compile. These were preparation bugs, not 38 client/bundler bugs.
- Preparation finishes with zero reported input/transpilation/Sass errors.
  Original client-web files and its two pre-existing dirty paths remain untouched.

Attempting the editor worker, analytics worker, data worker, mail, then the
combined four-entry workload using the release driver, native CSS auto-mount,
four preparation workers and local stage traces. Each invocation has a ten-minute
bound. Build and output syntax success would not establish full application
runtime correctness: workflow-specific defines, host globals, localization,
browser/worker execution and integration with the production loader still need
their own observations. Full-app output/runtime coverage is not yet established.

### First complete-input attempts

All five invocations reached build diagnostics; none published a complete app.
Times include traversing the complete staged input tree, even for a small entry.

| Entry | Elapsed | First observed failure |
| --- | ---: | --- |
| editor worker | 247.0 s | Vendor ESM `.js` distribution lacks module package mode |
| analytics worker | 223.5 s | Same format issue in a telemetry ESM distribution |
| data worker | 276.4 s | Same format issue in a utility ESM distribution |
| mail | 274.2 s | Annotation reader panics on a Unicode comment |
| all four entries | 277.3 s | Same Unicode comment panic |

The Unicode failure is a real extractor bug, not client/environment noise:
`SymbolTags::from_comments_parent` sliced a comment at byte 20 without checking
that it was a UTF-8 character boundary. Added a non-proprietary regression with
two-/three-byte characters crossing that boundary, ordinary short comments and
valid mixed-case annotations. It reproduces the exact panic before the fix.
Use checked prefix access, preserving existing ASCII case-insensitive matching.
The required spot-check passes after the fix, including all 76 AST tests and the
workspace check; rebuilt the release driver through `mbx`.

For the vendor format issue, preserve root package manifests and CJS
distributions. Audit distribution directories syntactically, and add local
nested `type: module` scopes only when every `.js` script in that scope is ESM.
Skip mixed/script directories and packages with private import maps rather than
blindly changing an entire dual-format package to ESM. This is an explicit local
corpus adaptation, not a claim that the original packages run unchanged.

The editor trace spent 246.5 s in the build but only 27.5 ms preparing its two
script jobs. Full-tree setup/inventory dominates this early failure. Analytics
had 405 preparation jobs and data worker 5,390; their traces are retained locally.
Large-root setup is already a bottleneck before testing linker scaling.

### Retry after the Unicode fix and vendor adaptation

The distribution audit checked 37,672 scripts, adding 934 local ESM scopes and
skipping 629 mixed/script candidates. It covers the three initial format failures
without changing root vendor manifests or original installed packages.

The editor worker now builds with native output: five source modules, four fresh
parses and one disk hit; all six emitted scripts pass syntax checks. No browser
worker runtime claim is made. Its instrumented run took 577.1 s. The completed
trace attributes 265.5 s to final `build.recheck`, but only 78 ms to preparation;
initial tree setup/inventory accounts for most of the other multi-minute gap.

The combined four-entry retry reached the ten-minute harness limit without a
diagnostic or completed publication. This is an imposed limit, not evidence of
a correctness failure. Its partial trace contains over 21,000 completed
preparation jobs, with extraction and resolution still open; the linker was not
demonstrated to complete. Aggregate worker-span times overlap and cannot be
summed as wall time. A forcibly terminated trace is truncated, not a valid
complete trace; only its complete event prefix was inspected.

Retrying the combined workload with a thirty-minute window. A separate
untraced control uses another output directory and another result file, avoiding
publication/result-file races. These overlapping diagnostic runs are not
benchmarks; the control distinguishes tracing overhead from inability to finish.
Full-app packaging and runtime correctness remain unverified.
