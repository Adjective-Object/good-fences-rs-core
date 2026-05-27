# Phase 03 – Resolver Decoupling: Working Notes

## Section 1: Introduce local `PathResolver` trait

- Created `import_resolver/src/resolve.rs` with `Resolution`, `PathResolver` trait, `TargetEnv`, `NODE_BUILTINS`.
- Used `String` for `Resolution.slug` instead of `oxc_span::CompactStr` — `CompactStr` is not
  re-exported from `oxc_span` at the pinned rev, and `oxc_str` is not a workspace dep. `String`
  is the fallback per PLAN.md. Can be revisited if `oxc_str` is added later.
- Added blanket impls for `&R`, `Box<R>`, `Arc<R>` to match swc's `Resolve` ergonomics.
- `TargetEnv` derives `Serialize`/`Deserialize` matching the swc version (serde already a dep).
- `NODE_BUILTINS` copied verbatim from `swc_ecma_loader-0.49.1`.

## Section 2: Rename `swc_resolver` → `node_resolver`

- `git mv` for directory, then renamed inner `node_resolver.rs` → `caching.rs` to avoid
  module-name collision with the parent directory.
- Updated all internal `use super::node_resolver::` → `use super::caching::` references.
- Updated `unused_finder` to import from `import_resolver::node_resolver::caching::` instead of
  `import_resolver::swc_resolver::node_resolver::`.

## Section 3: Replace `FileName`/`Resolve` with `&Path`/`PathResolver`

- Every `impl Resolve for X` became `impl PathResolver for X`.
- `FileName::Real(p)` match arms eliminated; `base` is now `&Path` directly.
- `Resolution { filename: FileName::Real(p), slug }` → `Resolution { path: p, slug }`.
- `tsconfig.rs`: `base_url_filename: FileName` → `base_url_path: PathBuf`.
- `caching.rs` (`resolve_filename`): returns `Result<PathBuf>` instead of `Result<FileName>`;
  `wrap()` returns `Result<PathBuf>`. Node builtins (`node:fs` etc.) now produce
  `PathBuf::from("node:fs")` — same string, different wrapper. Callers that only cared about
  the path string are unaffected.
- `pkgjson_rewrites.rs`: swapped `swc_common::collections::{AHashMap, AHashSet}` →
  `ahashmap::{AHashMap, AHashSet}`. Added `ahashmap` as a dependency of `import_resolver`.
- `common.rs`: type alias now points to `ahashmap::AHashMap`.

## Section 4: Update callers outside `import_resolver`

- `manual_resolver.rs`: `resolve_with_extension` now takes `base: &Path` and
  `resolver: impl PathResolver`. Removed all `FileName::Real(...)` wrapping and
  `match resolved.filename { FileName::Real(f) => ..., _ => ... }` destructuring.
- `unused_finder/src/parse/data.rs`: `resolve_hashmap`, `resolve_hashset`, `try_resolve`
  now use `&Path` + `impl PathResolver`. Removed `FileName::Real` wrapping/matching.
  The non-`Real` error branches were dead code (every resolution from our resolver
  already returned a `PathBuf`).
- `unused_finder/src/unused_finder.rs`: `SourceFiles::try_resolve` and
  `resolver_for_packages` now use `impl PathResolver` instead of `impl Resolve`.
  `TargetEnv` imported from `import_resolver::resolve` instead of `swc_ecma_loader`.

## Section 5: Drop swc deps from `import_resolver`

- Removed `swc_common`, `swc_ecma_loader`, `thiserror` from `import_resolver/Cargo.toml`.
- All 18 tests pass. Full workspace compiles and tests pass (only pre-existing
  `dockerfile_matches_rust_version` failure, unrelated).
