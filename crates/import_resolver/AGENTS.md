# import_resolver — agent notes

## Purpose

Resolves TypeScript/JavaScript import specifiers to real filesystem paths.
It handles node_modules resolution, tsconfig `paths`/`baseUrl` mappings,
`package.json` `exports`/`browser` fields, and provides an ftree-backed
incremental cache.

## PathResolver trait

The public interface for callers is the `PathResolver` trait in `src/resolve.rs`:

```rust
pub trait PathResolver: Send + Sync {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution>;
}

pub struct Resolution {
    pub path: PathBuf,
    pub slug: Option<CompactStr>,
}
```

`base` is the **directory** containing the file that owns the import.
`specifier` is the raw import string (e.g. `"./foo"`, `"react"`, `"@scope/pkg/sub"`).

Returns `Err` for node built-ins and truly unresolvable specifiers; callers
should treat `Err` as "skip this import" rather than a hard failure.

## No oxc_resolver

`oxc_resolver` is intentionally **not** used here.  The custom resolver has
tsconfig-paths handling, `package.json` browser/exports semantics,
ftree-backed caching, and a `mark_dirty_root` API used by `unused_finder`'s
incremental path.  Replacing it is out of scope.

## TargetEnv / NODE_BUILTINS

`TargetEnv` (Browser/Node) and `NODE_BUILTINS` are copied locally from the old
`swc_ecma_loader` source so this crate no longer depends on SWC.
