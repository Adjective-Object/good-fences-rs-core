High level goal: drop the remaining `swc_common` / `swc_atoms` dependencies
from `tag_graph`. Its production code is already swc-free; only tests touch
swc types.

Depends on phase 4 (for `oxc_span::CompactStr` and `oxc_span::Span` being
established as the workspace conventions).

## Update tests

[crates/tag_graph/src/lib.rs](crates/tag_graph/src/lib.rs#L629) uses
`swc_common::{BytePos, Span}` and `swc_atoms::Atom` only inside `#[cfg(test)]`
blocks.

- [x] Replace `use swc_common::{BytePos, Span};` with `use oxc_span::Span;`
- [x] Replace `Span::new(BytePos(a), BytePos(b))` constructors with `Span::new(a, b)`
- [x] Replace `use swc_atoms::Atom;` with `use oxc_span::CompactStr;`
- [x] Replace `Atom::from("...")` with `CompactStr::from("...")` (or `CompactStr::new`)
- [x] All existing assertions must pass unchanged

## Cargo.toml cleanup

- [x] Remove `swc_common` and `swc_atoms` from `tag_graph/Cargo.toml`
- [x] Add `oxc_span` (dev-dependency only if test-only)
- [x] `cargo test -p tag_graph` passes
