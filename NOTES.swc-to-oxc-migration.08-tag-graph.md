# Notes: swc-to-oxc-migration phase 08 — tag_graph

## Status

All tasks complete. The `tag_graph` crate was already swc-free when this phase
was audited.

## Findings

### swc references were pre-removed

The TODO was written anticipating that `crates/tag_graph/src/lib.rs` at line
629 would still have `use swc_common::{BytePos, Span}` and `use swc_atoms::Atom`
in `#[cfg(test)]` blocks. By the time this phase ran, those imports were already
replaced — the file uses `oxc_span::Span` and `compact_str::CompactString as
CompactStr` instead. No `swc_common` or `swc_atoms` entries appear anywhere in
`crates/tag_graph/`.

### `CompactStr` source: `compact_str` not `oxc_span`

The TODO plan targeted `oxc_span::CompactStr`, but `CompactStr` is NOT publicly
re-exported from `oxc_span` (it is used internally via `oxc_str`). The wider
codebase (`ast_segmenter`, `tag_graph`) already settled on importing directly
from `compact_str`:

```rust
use compact_str::CompactString as CompactStr;
```

This is consistent with `ast_segmenter`'s `VariableScope` API, which takes and
returns `compact_str::CompactString`. Using the same concrete type avoids any
newtype friction at the boundary.

### Cargo.toml dev-deps

`crates/tag_graph/Cargo.toml` uses `compact_str = "0.9.0"` as a dev-dep
(test-only) and `oxc_span.workspace = true` as a dev-dep (for `Span`). No
production-code dep on either; no swc crates at all.

### Tests

16 tests, all pass. `cargo test -p tag_graph` clean.  Full workspace
`cargo check` also clean (only pre-existing warnings about napi_derive cfg).

## Verification

```
./scripts/spot-check  →  All spot checks passed!
cargo test -p tag_graph  →  16 passed; 0 failed
```
