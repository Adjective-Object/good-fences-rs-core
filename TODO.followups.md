# Follow-up items

## Pre-existing warnings in workspace

- `ftree_cache`: elided lifetime warning (`RwLockReadGuard<'_, Option<T>>`)
- `unused_finder`: dead code warnings for `ErrList`, `FileInfo`, `ExportedItem`

## Pre-existing test failure

- `repo-health::dockerfile_matches_rust_version` fails: Dockerfile rust channel
  `nightly-2024-10-25` doesn't match `rust-toolchain.toml` channel `nightly-2026-05-01`
