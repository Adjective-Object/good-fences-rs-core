# import_resolver

## PathResolver trait

Resolution is abstracted behind a `PathResolver` trait (`src/resolve.rs`):

```rust
pub trait PathResolver: Send + Sync {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution>;
}
```

Blanket impls exist for `&R`, `Box<R>`, and `Arc<R>`.

Implementors:
- `node_resolver::CachingResolver` — production resolver with ftree-backed
  caching, tsconfig-paths, package.json browser/exports semantics.
- `manual_resolver::ManualResolver` — test double.

## Resolver lineage

Previously `node_resolver/` was `swc_resolver/` and consumed
`swc_ecma_loader` traits. The swc dependency was replaced with the local
`PathResolver` trait; `swc_common::FileName` was replaced with `&Path`.

## Not using oxc_resolver

The custom resolver has tsconfig-paths handling, ftree caching, and
`mark_dirty_root` for incremental invalidation. Replacing it with
`oxc_resolver` is out of scope.
