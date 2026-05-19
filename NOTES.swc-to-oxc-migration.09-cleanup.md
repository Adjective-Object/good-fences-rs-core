# NOTES — swc-to-oxc-migration phase 9 (cleanup)

## Dead helper crates

`swc_utils_parse` and `swc_utils_print` (package name `normalize_src`) had zero
dependents outside themselves.  Deleted by removing the directories; the
workspace `members = ["crates/*"]` glob drops them automatically — no
`Cargo.toml` edits needed for membership.

## swc-compat adapter

`WrapFileLogger::from_swc_source_file` and `swc_span_to_oxc` lived behind
`#[cfg(feature = "swc-compat")]` in `logger_srcfile`.  No external caller
enabled this feature (confirmed by grepping all `Cargo.toml` files for
`swc-compat`).  Deleted the `swc_compat_impl` module, the feature gate, the
optional `swc_common` dep, and the gated round-trip test.

## Workspace swc dependencies

All seven SWC workspace deps removed from root `Cargo.toml`.  After a
`cargo check`, `Cargo.lock` contains zero `swc_*` entries and no
`normalize_src` entry — confirmed.

## Nightly feature gates

`box_patterns` was already gone (removed in earlier phases).  The remaining
nightly gates in the workspace are:

| crate             | feature                                    |
|-------------------|--------------------------------------------|
| `ftree_cache`     | `adt_const_params`, `unsized_const_params` |
| `patch_vfs`       | `str_split_remainder`, `closure_lifetime_binder` |
| `import_resolver` | `iterator_try_collect`                     |

All are pre-existing and unrelated to the SWC → OXC migration; left as-is.

## cargo-shear

`cargo-shear` is not installed in this environment.  Skipped the automated
unused-dep scan; manual inspection of all `Cargo.toml` files found no
remaining SWC references.

## NOTES.swc-to-oxc-migration.md

This file does not exist in the repo — nothing to archive.

## Bench + regression

`cargo bench --workspace --no-run` succeeds (targets exist in `test_tmpdir`,
`tsconfig_paths`, `unused_finder`, `unused_finder_napi`).  No algorithmic
changes were made in this phase (pure dead-code deletion), so order-of-magnitude
regressions are not expected.  Full bench run skipped per the plan ("eyeball for
order-of-magnitude regressions" only).
