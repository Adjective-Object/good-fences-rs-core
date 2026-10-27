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
Evaluate nested condition trees recursively in order. Resume later parent
conditions only when a nested branch has no match, never after a private leaf.
Apply exact one-step aliases before builtin/external policy and downstream Node
builtin resolution. Do not expand prefixes or recurse through aliases. Resolved
stubs remain real files subject to consumer inventory/graph validation.
Explicit .mjsx/.cjsx requests may resolve .mtsx/.ctsx, matching .mjs/.cjs
TypeScript substitution; do not widen default extension probing implicitly.
Keep legacy `.js` preference explicit in `prefer_js_extension`; ordered custom
suffix policies disable it. Include both suffix order and preference in target keys.
Browser defaults activate browser. Bare builtin spellings resolve installed
packages through normal checked import/require/exports and cache proofs, excluding
the legacy cwd-relative file shortcut. Never allow cwd shadowing, external escape
or implicit polyfills. Reject remaining node: requests, missing builtin packages
and native addons. Preserve original importer/request in errors.
Apply a dependency's browser object mapping to its selected legacy package entry,
then retain the importer's request/path mapping policy.
Rebase rooted metadata file paths through `packagejson::package_relative_path`
before package joins/target validation, including browser file keys. Preserve
bare request keys/redirects and direct absolute imports; inventory still admits
the resolved file through its ordinary checked route.
An existing exports map forbids filesystem fallback for unmatched/private paths.
Bare-string exports normalize to root plus "./*":"./*" for arbitrary package
files (intentional Node deviation); explicit maps keep encapsulation. Reject
request subpaths that traverse outside the package root.

Scoped resolution proofs use explicit request-owned observers at actual reads.
Cache metadata, browser-derived probes and positive/negative package targets
with their original provenance; never restat an old value to invent its inputs.
Tracked calls reject masked IO errors. Consumers key request mode/policy and
freshly validate the returned proof before reuse/success, then invalidate on
change. Ordinary candidates seal kind/identity/links, not source bytes; only
content-dependent manifests seal bytes. Do not enumerate directory inventories.

Filesystem observations belong to an explicit resolver epoch. Share exact
manifest bytes across request modes, but keep target/condition/policy results
distinct. Initialize context and target cells outside DashMap locks; wrap cached
paths using their original canonical observations. Invalidation replaces the
snapshot and cache trees together, including relative-file misses.

Resolve effective # requests before cwd/package probes. Nearest observed manifest
owns imports even when empty; stop scope search at node_modules/root. Share the
ordered exports target evaluator without its legacy string/directory extensions.
Local targets bypass public exports; dependency redirects use owning-package
checked resolution, never aliases again or external escape. Bound # recursion.
Retain scope misses/manifest bytes and selected target/dependency proofs; no
separate imports cache or reconstructed provenance.
Apply the shared browser path rewrite in the imports owner's scope after target
resolution, retaining redirected dependencies' legacy-entry browser mapping.
