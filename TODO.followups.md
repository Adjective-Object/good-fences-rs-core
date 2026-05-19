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

## `scope_from_semantic` includes symbols from nested scopes (swc-to-oxc phase 4)

The `scope_from_semantic` shim in `ast_name_tracker` (added in phase 4) uses span containment
to find symbols belonging to a statement. This includes symbols declared in nested scopes
(e.g., inside function bodies). A proper implementation would walk only the statement's
direct scope boundary. Phase 5 should replace this with a precise scope-tree walk or
remove it entirely once `ast_name_tracker` is fully migrated.

## `segment_graph.rs` retains `swc_atoms::Atom` for `Name` (swc-to-oxc)

`crates/ast_segmenter/src/segment_graph.rs` still uses `swc_atoms::Atom` as the `Name`
type (it did not need to be changed for phase 4). Migrate this to a plain `Arc<str>` or
`Box<str>` in phase 8/9 cleanup alongside the `ast_name_tracker` name type migration.

## `unused_finder/parse/exports_visitor_tests.rs` still parses with swc (swc-to-oxc)

The test helper in `crates/unused_finder/src/parse/exports_visitor_tests.rs` still uses
the swc parser to produce `RawImportExportInfo` for assertion. This will be migrated as
part of the `unused_finder` phase (phase 5 or later).
