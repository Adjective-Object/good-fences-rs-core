# Notes: swc-to-oxc-migration phase 00-scaffolding

## Toolchain upgrade required

The OXC rev `87f065eb` (oxc v0.132.0) requires `rustc >= 1.93.0` and Cargo
support for `edition = "2024"` (used by `tasks/website_linter` in the OXC
repo). The previous nightly (`nightly-2024-10-25`, rustc 1.84.0) was too old.

Updated `rust-toolchain.toml` to `nightly-2026-05-01` (rustc 1.97.0).

## serde version pin

`cargo generate-lockfile` resolved serde to 1.0.228, which removed the
`serde::__private` module. `swc_common v0.37.5` and `swc_config v0.1.15`
depend on `serde::__private`. Pinned serde to `>=1.0.117, <1.0.219` in
workspace dependencies to keep the swc stack compiling during coexistence.
This pin can be removed in phase 9 (cleanup) when the swc deps are dropped.

## `leading_comments_at` design decision

**Rule chosen:** A comment is "leading" for a statement if the gap between
the comment's `span.end` and the statement's `span.start` (or the next
comment's `span.start`) contains only whitespace characters. Comments are
collected greedily backwards.

Consequence: an interior comment between two statements is attributed as
leading to the *second* statement (the one it immediately precedes). This
matches the intuitive reading of `// @ALLOW-UNUSED-EXPORT` placed on the line
before an export.

## `oxc_utils_parse` API

- No wrapper struct — callers pass `&Allocator`, get back `ParserReturn<'a>`.
- `parse_file` infers `SourceType` from the path extension, falls back to `.ts`.
- `parse_ts` / `parse_tsx` are convenience wrappers for callers without a path.
- `leading_comments_at` takes `&[Comment]` + source text + offset. Uses
  `partition_point` for O(log n) initial lookup, then linear backward walk.
