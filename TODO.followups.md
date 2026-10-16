# Follow-up items

## Pre-existing warnings in workspace

- `ftree_cache`: elided lifetime warning (`RwLockReadGuard<'_, Option<T>>`)
- `unused_finder`: dead code warnings for `ErrList`, `FileInfo`, `ExportedItem`

## Style: thiserror usage in unused_finder

- `unused_finder/src/parse/exports_visitor_runner.rs` uses `thiserror::Error`;
  repo convention is `eyre`. Low priority — the type is internal.

## Pre-existing test failure

- `repo-health::dockerfile_matches_rust_version` fails: Dockerfile rust channel
  `nightly-2024-10-25` doesn't match `rust-toolchain.toml` channel `nightly-2026-05-01`

## cargo-shear: pre-existing unused dependencies

`cargo shear` reports 25 unused deps across the workspace — none are swc-related.
Notable ones:
- `hashbrown` unused in `deadlock_ref` and `ftree_cache`
- `oxc_span` unused in `good_fences`
- `serde_derive` unused in `good_fences`, `import_resolver`, `packagejson`
- `itertools` unused in `unused_finder`
- `unused_finder/src/throttled_async_map.rs` is an unlinked file
- Various napi crates have unused `napi`, `anyhow`, `js_err`, `serde` deps
