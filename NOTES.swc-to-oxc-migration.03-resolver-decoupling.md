# Notes: swc→oxc migration phase 03 — resolver decoupling

## What was done

Removed `swc_common::FileName` and `swc_ecma_loader::{resolve::Resolve, TargetEnv}` from
`import_resolver` and its callers. Introduced a local `PathResolver` trait whose `resolve`
method takes `&Path` directly, eliminating the `FileName::Real(p) => p` unwrap dance
that every implementation had to repeat.

## Design decisions

### `Resolution { path: PathBuf, slug: Option<CompactStr> }`
`path` is always a real filesystem `PathBuf`. There is no representation for external or
builtin modules — callers that receive an `Err` from a builtin now skip it or log it.
`slug` mirrors the original `swc_ecma_loader::resolve::Resolution::slug` and is always
`None` for now; reserved for future use (e.g. package name tagging).

### `CompactStr` source
`oxc_span::CompactStr` is a re-export from `oxc_str` but was not pub-exported in the
workspace's pinned oxc rev. Used `compact_str::CompactString` (the upstream crate, already
a transitive dep) aliased as `CompactStr` instead, to avoid reaching into a private
internal path.

### Node builtins: `bail!`, not `FileName::Custom`
The old code returned `FileName::Custom("node:...")` for built-in modules (e.g. `fs`, `path`).
Callers then had to special-case `FileName::Custom` branches. The new code calls
`bail!("node builtin: {}", module_specifier)` in `caching.rs`. Downstream error handlers
(in `manual_resolver.rs`) detect this with `e.to_string().starts_with("node builtin:")`.
`data.rs::resolve_hashmap`/`resolve_hashset` already treated non-`FileName::Real` results
as resolution errors — this preserves that behavior without requiring special cases.

### `base_url_filename` removed from `ProcessedTsconfigPaths`
The field was always `FileName::Real(base_url.clone())` — a redundant copy. Replaced all
uses with `self.tsconfig.base_url.as_path()` directly.

### `caching.rs` (formerly `node_resolver.rs`)
Rust cannot have both a directory `node_resolver/` and a file `node_resolver.rs` in the
same module scope. The inner file was renamed to `caching.rs` and the module declaration
changed to `pub mod caching;`. The public type `CachingNodeModulesResolver` is re-exported
from `node_resolver/mod.rs`.

### `swc_common::collections::AHashMap` → `ahashmap::AHashMap`
`pkgjson_rewrites.rs` and `common.rs` used `swc_common::collections::AHashMap`. This was
replaced with `ahashmap::AHashMap` (a local path crate already used throughout the workspace).

### Blanket `impl PathResolver for &T`
Added in `resolve.rs` so that callers can pass `&resolver` without boxing:
```rust
impl<T: PathResolver> PathResolver for &T { ... }
```
This lets `resolve_hashmap`/`resolve_hashset` in `data.rs` accept `&impl PathResolver`
even when the concrete resolver is not `Copy`.

## Gotchas

- `pkgjson_rewrites.rs` had `use super::node_resolver::CachingNodeModulesResolver` (referring
  to the old file name). Updated to `use super::caching::CachingNodeModulesResolver`.
- `unused_finder.rs` still had `impl Resolve + Sync` on `SourceFiles::try_resolve` — easy
  miss since only the import lines were updated in the first pass.
- `mod caching` needed to be `pub mod caching` so that external crates importing
  `import_resolver::node_resolver::caching::*` can still access the types.
