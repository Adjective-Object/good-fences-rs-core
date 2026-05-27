# Phase 09 Cleanup — Working Notes

## Delete dead helper crates

- Deleted `crates/swc_utils_parse/` and `crates/swc_utils_print/`.
- No references remained in `crates/` — all consumers were migrated in earlier phases.
- Workspace uses `members = ["crates/*"]` glob, so no Cargo.toml workspace member edits needed.
- Pre-existing warnings in `unused_finder` (dead code: `ErrList`, `ExportedItem`, `FileInfo`) are unrelated.
