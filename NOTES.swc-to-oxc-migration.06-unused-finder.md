# Notes: swc-to-oxc-migration.06-unused-finder

## State at start of this phase

All swc source-code references had already been removed from the `unused_finder`
crate in earlier phases.  The only remaining work was:

1. The per-call `Allocator::default()` in `exports_visitor_runner.rs` needed to
   become a **thread-local arena** (`PARSE_ARENA`) as specified in the plan.
2. Lingering swc Cargo deps (`swc_common`, `swc_ecma_parser`, `swc_utils_parse`)
   needed to be dropped.
3. The `swc-compat` feature flag on the `logger_srcfile` dep needed to be removed
   (nothing in this crate used `swc_span_to_oxc`).

## Key decisions

### Thread-local PARSE_ARENA

`PARSE_ARENA: RefCell<Allocator>` is declared at module scope in
`exports_visitor_runner.rs`.  Each `get_file_segments` call borrows it,
parses + segments the file, extracts fully-owned output (`Vec<Segment>`), drops
all arena-lifetime values, then calls `arena_cell.borrow_mut().reset()`.

The borrow-then-drop dance is necessary because `reset()` takes `&mut self`
while the parse borrows `&self`.  The two borrows must not overlap — we
explicitly `drop(arena)` (the shared borrow) before calling
`arena_cell.borrow_mut().reset()`.

### Arena-reuse unit test

The test creates a small `.ts` file, calls `get_file_segments` twice on the
same thread, and asserts that `Allocator::capacity()` does not increase between
the two calls.  `capacity()` delegates to `bumpalo::Bump::allocated_bytes()` and
represents the total backing memory held by the arena — a stable value confirms
that `reset()` returned chunks to the pool rather than discarding them.

### `swc-compat` feature removed

`logger_srcfile`'s `swc-compat` feature gates an `swc_span_to_oxc` helper.
Since `unused_finder` never called it, removing the feature annotation avoids
pulling `swc_common` into the crate's dependency tree through a back door.

## Gotchas

- `test_tmpdir::TmpDir` exposes `.root()` (not `.path()`) to get the temp
  directory's canonical path.  The `capacity()` method (not `allocated_bytes()`)
  is the public API on `oxc_allocator::Allocator`.
