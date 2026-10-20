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
- `node_resolver::MonorepoResolver` — production resolver with ftree-backed
  caching, tsconfig export conditions and package.json browser/exports semantics.
- `manual_resolver::ManualResolver` — test double.

## Resolver lineage

Previously `node_resolver/` was `swc_resolver/` and consumed
`swc_ecma_loader` traits. The swc dependency was replaced with the local
`PathResolver` trait; `swc_common::FileName` was replaced with `&Path`.

## Not using oxc_resolver

The custom resolver has export-condition handling, ftree caching, and
`mark_dirty_root` for incremental invalidation. Replacing it with
`oxc_resolver` is out of scope.

## Structural graph policy

`StructuralResolver` uses separate import/require cache trees. Package target
keys include the active condition set, including nearest tsconfig conditions.
Never apply cached-resolver baseUrl/paths rewrites; the legacy manual fence
resolver remains independent. Externals require exact
explicit declarations. `invalidate` replaces both trees, including misses;
call it after filesystem/config changes, then refresh the graph. Symlink
identity follows explicit options; source paths must follow that same policy.
Export conditions are active sets, not priority lists; preserve package JSON
declaration order. Node defaults include node/node-addons/module-sync, not browser
defaults. Select a subpath before evaluating its conditions.
An existing exports map forbids filesystem fallback for unmatched/private paths.
Bare-string exports normalize to root plus "./*":"./*" for arbitrary package
files (intentional Node deviation); explicit maps keep encapsulation. Reject
request subpaths that traverse outside the package root.
