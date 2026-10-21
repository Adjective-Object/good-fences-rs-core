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

Pending recovery and integration of the skipped dirty `perf-optim` edits.
The base already contains typed wire tables and borrowed syntax/Ecco adapters,
but the production sidecar record still uses CBOR.

## Client-web exercise

Pending publication fixes and completed FlatBuffers integration. Start with
isolated small packages, increase dependency graph size, then attempt the full
mail workload with all application and worker entries. Distinguish preparation,
resolution, build, output and runtime failures; never count a successful
transpilation or build as runtime correctness.
