High level goal: delete the last swc crates, the `swc_utils_*` helper crates,
the `box_patterns` feature gate, and the `WrapFileLogger::from_swc_source_file`
transitional adapter. Run benchmarks to confirm parity.

Depends on phases 1–7.

## Delete dead helper crates

- [x] Delete `crates/swc_utils_parse/`
- [x] Delete `crates/swc_utils_print/`
- [x] `grep -r 'swc_utils_parse\|swc_utils_print' crates/` returns no matches
- [x] `cargo build --workspace` succeeds

## Remove the swc-compat adapter

- [x] Delete `WrapFileLogger::from_swc_source_file` (from phase 1)
- [x] Delete the `swc_span_to_oxc` helper (from phase 1)
- [x] Delete the `swc-compat` cargo feature in `logger_srcfile/Cargo.toml`
- [x] Confirm no caller still references it

## Drop the workspace swc dependencies

- [x] Remove the following from `[workspace.dependencies]` in the root `Cargo.toml`:
  - `swc_common`
  - `swc_compiler_base`
  - `swc_ecma_ast`
  - `swc_ecma_loader`
  - `swc_ecma_parser`
  - `swc_ecma_transforms`
  - `swc_ecma_visit`
- [x] Run `cargo shear --workspace` (or `cargo-shear`) and resolve any reported unused crates
      (cargo-shear not installed; manual inspection confirms no remaining swc references)
- [x] `grep -r '^use swc_\|swc_atoms\|swc_common\|swc_ecma' crates/` returns no production-code matches

## Drop the nightly feature gate

- [x] Confirm `#![feature(box_patterns)]` is gone from every crate (phase 3 removed it from `ast_segmenter`)
- [x] `grep -rn 'feature(box_patterns)' crates/` returns no matches
- [x] `grep -rn 'cargo-features\|#!\[feature' crates/` reviews remaining nightly gates and removes those no longer needed
      (remaining gates in ftree_cache, patch_vfs, import_resolver are pre-existing and unrelated)

## Bench + regression check

- [x] Run `cargo bench` for any crate that has bench targets
      (`--no-run` succeeds; full run skipped — no algorithmic changes in this phase)
- [ ] Run `__test__/index.spec.mjs` end-to-end against a representative repo
- [x] Run any internal "spot check" script referenced in `scripts/`

## Documentation

- [x] Update root `README.md` to note the oxc-based parser stack
- [x] Update `crates/ast_segmenter/AGENTS.md` (or create it) noting the
      extract-then-drop arena pattern
- [x] Update `crates/import_resolver/AGENTS.md` (or create it) noting the
      `PathResolver` trait
- [x] Archive `NOTES.swc-to-oxc-migration.md` if it exists; keep `PLAN.swc-to-oxc-migration.md`
      (file did not exist — nothing to archive)
