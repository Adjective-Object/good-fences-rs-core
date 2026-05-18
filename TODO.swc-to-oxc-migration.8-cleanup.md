High level goal: delete the last swc crates, the `swc_utils_*` helper crates,
the `box_patterns` feature gate, and the `WrapFileLogger::from_swc_source_file`
transitional adapter. Run benchmarks to confirm parity.

Depends on phases 1–7.

## Delete dead helper crates

- [ ] Delete `crates/swc_utils_parse/`
- [ ] Delete `crates/swc_utils_print/`
- [ ] `grep -r 'swc_utils_parse\|swc_utils_print' crates/` returns no matches
- [ ] `cargo build --workspace` succeeds

## Remove the swc-compat adapter

- [ ] Delete `WrapFileLogger::from_swc_source_file` (from phase 1)
- [ ] Delete the `swc-compat` cargo feature in `logger_srcfile/Cargo.toml`
- [ ] Confirm no caller still references it

## Drop the workspace swc dependencies

- [ ] Remove the following from `[workspace.dependencies]` in the root `Cargo.toml`:
  - `swc_common`
  - `swc_compiler_base`
  - `swc_ecma_ast`
  - `swc_ecma_loader`
  - `swc_ecma_parser`
  - `swc_ecma_transforms`
  - `swc_ecma_visit`
- [ ] Run `cargo shear --workspace` (or `cargo-shear`) and resolve any reported unused crates
- [ ] `grep -r '^use swc_\|swc_atoms\|swc_common\|swc_ecma' crates/` returns no production-code matches

## Drop the nightly feature gate

- [ ] Confirm `#![feature(box_patterns)]` is gone from every crate (phase 3 removed it from `ast_segmenter`)
- [ ] `grep -rn 'feature(box_patterns)' crates/` returns no matches
- [ ] `grep -rn 'cargo-features\|#!\[feature' crates/` reviews remaining nightly gates and removes those no longer needed

## Bench + regression check

- [ ] Run `cargo bench` for any crate that has bench targets
- [ ] Compare wall-clock and allocation counts (if tracked) against a pre-migration baseline
- [ ] Run `__test__/index.spec.mjs` end-to-end against a representative repo
- [ ] Run any internal "spot check" script referenced in `scripts/`

## Documentation

- [ ] Update root `README.md` to note the oxc-based parser stack
- [ ] Update `crates/ast_segmenter/AGENTS.md` (or create it) noting the
      extract-then-drop arena pattern
- [ ] Update `crates/import_resolver/AGENTS.md` (or create it) noting the
      `PathResolver` trait
- [ ] Archive `NOTES.swc-to-oxc-migration.md` if it exists; keep `PLAN.swc-to-oxc-migration.md`
