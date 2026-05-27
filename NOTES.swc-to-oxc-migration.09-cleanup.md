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

## Drop the workspace swc dependencies

- Removed all 7 `swc_*` entries from `[workspace.dependencies]` in root `Cargo.toml`:
  `swc_common`, `swc_compiler_base`, `swc_ecma_ast`, `swc_ecma_loader`,
  `swc_ecma_parser`, `swc_ecma_transforms`, `swc_ecma_visit`.
- No crate-level `Cargo.toml` under `crates/` referenced any `swc_*` dependency.
- `grep -r '^use swc_\|swc_atoms\|swc_common\|swc_ecma' crates/` returns only
  comments (historical attribution in `import_resolver/src/node_resolver/`).
- `cargo shear` reports no swc-related unused deps. Other findings (hashbrown,
  serde_derive, etc.) are pre-existing — logged in `TODO.followups.md`.
- `cargo build --workspace` and `./scripts/spot-check` both pass.

## Drop the nightly feature gate

- `#![feature(box_patterns)]` was already removed in phase 3 from `ast_segmenter`; no other crate ever used it.
- `grep -rn 'feature(box_patterns)' crates/` returns zero matches — confirmed clean.
- Remaining nightly gates are all actively used and unrelated to the swc migration:
  - `patch_vfs`: `str_split_remainder` (`.remainder()` call), `closure_lifetime_binder` (`for<'a>` closure syntax)
  - `import_resolver`: `iterator_try_collect` (`.try_collect()` in tsconfig.rs)
  - `ftree_cache`: `adt_const_params`, `unsized_const_params` (const generic parameters)
- These are pre-existing and cannot be removed without rewriting unrelated code.
