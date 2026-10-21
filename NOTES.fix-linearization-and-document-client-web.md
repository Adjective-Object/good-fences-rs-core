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
