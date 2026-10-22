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
  caching, explicit export conditions and package.json browser/exports semantics.
The separate `manual_resolver` classifies imports for fence checks without
compiler path aliases; it is not a `PathResolver` implementation.

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
keys include the explicit active condition set. Never discover or interpret
tsconfig files, including customConditions/customSourceConditions or
extends/references; no baseUrl/paths rewrites in either resolver.
Conditions come from NodeModulesResolverOptions::export_conditions.
Externals require exact
explicit declarations. `invalidate` replaces both trees, including misses;
call it after filesystem/config changes, then refresh the graph. Symlink
identity follows explicit options; source paths must follow that same policy.
Export conditions are active sets, not priority lists; preserve package JSON
declaration order. Node defaults include node/node-addons/module-sync, not browser
defaults. Select a subpath before evaluating its conditions.
Apply exact one-step aliases before builtin/external policy and downstream Node
builtin resolution. Do not expand prefixes or recurse through aliases. Resolved
stubs remain real files subject to consumer inventory/graph validation.
Explicit .mjsx/.cjsx requests may resolve .mtsx/.ctsx, matching .mjs/.cjs
TypeScript substitution; do not widen default extension probing implicitly.
Browser defaults activate browser; structural browser requests reject remaining
Node builtins even with explicit external policy and native addons, never inject
polyfills/empty stubs. Preserve original importer/request in errors.
Apply a dependency's browser object mapping to its selected legacy package entry,
then retain the importer's request/path mapping policy.
An existing exports map forbids filesystem fallback for unmatched/private paths.
Bare-string exports normalize to root plus "./*":"./*" for arbitrary package
files (intentional Node deviation); explicit maps keep encapsulation. Reject
request subpaths that traverse outside the package root.
