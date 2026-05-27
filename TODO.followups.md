# Follow-up items

## Pre-existing warnings in workspace

- `ftree_cache`: elided lifetime warning (`RwLockReadGuard<'_, Option<T>>`)
- `unused_finder`: dead code warnings for `ErrList`, `FileInfo`, `ExportedItem`

## Pre-existing test failure

- `repo-health::dockerfile_matches_rust_version` fails: Dockerfile rust channel
  `nightly-2024-10-25` doesn't match `rust-toolchain.toml` channel `nightly-2026-05-01`
- `unused_finder::test::test_typeonly_interface_allowed` fails: expected
  `extra_symbol_tags` to contain `[TypeOnly]` but got `[Entry, TypeOnly]`.
  Likely a tag-propagation issue in the BFS traversal (re-exported type-only
  symbols gaining an `Entry` tag).
