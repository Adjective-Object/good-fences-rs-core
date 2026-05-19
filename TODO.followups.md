# Follow-up TODOs

## Avoid bare `cargo update` while SWC and OXC coexist (swc-to-oxc)

`serde 1.0.220+` removed `serde::__private`, which `swc_common 0.37.4` depends on.
A bare `cargo update` will bump serde and break the build until `swc_common` is removed
in Phase 8. Document this in the contributing guide or add a CI check.

## nightly-only `box_patterns` in ast_segmenter

`crates/ast_segmenter/src/lib.rs` uses `#![feature(box_patterns)]`, which keeps the project
on nightly. Phase 9 (cleanup) should convert box patterns to manual destructuring or switch
to `let-else` + `match` to allow a stable toolchain.

## `unused_finder_napi` cfg warnings on newer toolchain

After upgrading to `nightly-2026-03-24`, `unused_finder_napi` emits ~10 warnings of the
form `unexpected cfg condition value: noop` from the `#[napi]` macro. These are upstream
(napi-derive crate) and unrelated to our changes. Track whether napi-derive has a fix.

## `repo-health` test failure: Dockerfile Rust version mismatch

`test_dockerfile_rust_version_matches_root_toml` in `repo-health` fails on the current branch
(pre-existing, not introduced by the logger migration). The Dockerfile's toolchain version
is out of sync with `rust-toolchain.toml`. Needs a Dockerfile update.

OXC populates `Comment::attached_to` with the start offset of the token the leading comment
is attached to. Once that semantic is confirmed stable, `leading_comments_at` could be
simplified to a filter on `c.is_leading() && c.attached_to == statement_start` (O(log n)
via partition_point is fine as-is, but the intent would be clearer).

## `segment_graph.rs` retains `swc_atoms::Atom` for `Name` (swc-to-oxc)

`crates/ast_segmenter/src/segment_graph.rs` still uses `swc_atoms::Atom` as the `Name`
type. Migrate this to a plain `Arc<str>` or `Box<str>` in phase 8/9 cleanup.

## `oxc_span::CompactStr` is not publicly exported (swc-to-oxc plan inaccuracy)

`PLAN.swc-to-oxc-migration.md` and TODO phase 8 describe migrating to
`oxc_span::CompactStr`, but that type is not re-exported from `oxc_span` at the
pinned revision (87f065e). The codebase settled on `compact_str::CompactString`
imported directly, which is consistent across all crates. Update the plan
documentation in Phase 9 cleanup.

## `ast_segmenter/src/variables.rs` unused doc-comment warning

`variables.rs:107` has a `///` doc comment on a closure/let-statement, producing
`#[warn(unused_doc_comments)]`. Convert to `//` plain comment during Phase 9 cleanup.

## `.js` files with TypeScript syntax require TS parse mode (swc-to-oxc phase 4 oversight)

`oxc_utils_parse::parse_file` was using `SourceType::from_path` which sets JavaScript
mode for `.js` files, rejecting TypeScript syntax (`export type`, interfaces, etc.)
common in this codebase. Fixed in phase 5 to always use TypeScript mode. Tracked here
in case other callers of `SourceType::from_path` exist.
