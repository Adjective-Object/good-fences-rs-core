# Phase 09 Cleanup — Working Notes

## Delete dead helper crates

- Deleted `crates/swc_utils_parse/` and `crates/swc_utils_print/`.
- No references remained in `crates/` — all consumers were migrated in earlier phases.
- Workspace uses `members = ["crates/*"]` glob, so no Cargo.toml workspace member edits needed.
- Pre-existing warnings in `unused_finder` (dead code: `ErrList`, `ExportedItem`, `FileInfo`) are unrelated.

## Remove the swc-compat adapter

- Deleted `swc_compat` module from `logger_srcfile/src/lib.rs` (contained `swc_span_to_oxc`, `from_swc_source_file`, `from_swc_source_map`).
- Deleted `swc_compat_tests` test module and its `swc_span_to_oxc_roundtrip` test.
- Removed `swc-compat` feature and `swc_common` optional dep from `logger_srcfile/Cargo.toml`.
- Removed `features = ["swc-compat"]` from `ast_segmenter/Cargo.toml` — it was the only consumer and didn't actually use any symbol from the module.
- `grep -r 'swc_compat\|from_swc_source\|swc_span_to_oxc' crates/` returns no matches.
