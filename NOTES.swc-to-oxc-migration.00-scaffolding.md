# Working notes — swc-to-oxc-migration Phase 0: scaffolding

## Rust toolchain upgrade

OXC rev `87f065ebf7cde21d1229322f4c7ee58baca5648e` uses `edition = "2024"` (stabilised in Rust
1.85.0, Feb 2025) and `rust-version = "1.93.0"`. The previous toolchain pin
(`nightly-2024-10-25` = Rust 1.84.0-nightly) did not support edition 2024, so the toolchain
was bumped to `nightly-2026-03-24` (Rust 1.96.0-nightly).

The `box_patterns` feature used by `ast_segmenter` is still available on that nightly.

## Lock-file surgery

`cargo update` (broad) bumped `serde_json` to 1.0.149 which in turn required `serde ≥ 1.0.220`.
`serde 1.0.220+` restructured internals via a new `serde_core` dependency and dropped the
public `serde::__private` module. `swc_common 0.37.4` depends on that private module, causing
a compile error.

Fix: run `cargo update` **narrowly** — update only the packages that OXC's new deps
actually conflict on, leaving `serde` and `serde_json` at their original versions:

```
cargo update allocator-api2 memchr bitflags proc-macro2 percent-encoding unicode-segmentation "syn:2"
```

This resolved all OXC version conflicts and added all OXC crates to the lock file without
touching serde (stays at 1.0.204) or serde_json (stays at 1.0.122).

**Avoid running bare `cargo update` in future phases** until swc_common is removed (phase 8),
as it will bump serde and break the build.

## OXC workspace dep alignment

All seven OXC deps use the same git URL + rev. The crates are included in the OXC mono-repo
so they share a single git fetch. No `features` are enabled at the workspace level; individual
crates opt-in via `crate.workspace = true` in their own `Cargo.toml`.

`oxc_ast_visit`, `oxc_semantic`, and `oxc_diagnostics` are added now (phase 0) even though
`oxc_utils_parse` itself doesn't use them directly — later phases will need them and having
all seven in `[workspace.dependencies]` avoids needing to touch root `Cargo.toml` again.

## Comment sort order

The PLAN states comments are "sorted by `span.end`". OXC's `trivia.rs` actually sorts by
`span.start`, and uses `partition_point` on `span.start`. For non-overlapping comments
(all real comments), sorted-by-start == sorted-by-end, so `partition_point(|c| c.span.end <=
statement_start)` is a valid binary search. This is documented in the `leading_comments_at`
doc comment.

## Leading-comment rule

Two tests cover the boundary: a blank-line gap means the comment leads neither statement;
no blank line means it leads the immediately following one. Rule: gap between comment end
and statement start (or next comment start) must be whitespace-only AND have < 2 `\n`
characters. This is `is_attached_gap` in `leading_comments.rs`.

OXC also tracks `CommentPosition::Leading / Trailing` on each comment. We respect that flag
by trimming trailing comments from the candidate set before the backward walk.

## parse_ts / parse_tsx module mode

`SourceType::ts()` and `SourceType::tsx()` default to `Unambiguous` module kind. We call
`.with_module(true)` to match the SWC `parse_typescript_module()` behavior that the existing
codebase relied on.

## `leading_comments_at` signature

The PLAN shows the signature as `(comments, source, statement_start)` in some places and
without `source` in others. We use the three-argument form because blank-line detection
requires slicing the source text.

## Alternative: use `comment.attached_to`

OXC's `Comment::attached_to` field contains the `span.start` of the token the comment is
attached to (for leading comments). A future simplification could replace the `partition_point`
+ backward walk with a simple filter:
`comments.iter().filter(|c| c.is_leading() && c.attached_to == statement_start)`.
Deferred to avoid relying on an `attached_to` semantic that OXC marks as not yet computed
for trailing comments.
