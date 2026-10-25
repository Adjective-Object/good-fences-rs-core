# import_resolver

Resolves Node/package requests using the repository's cached resolver.
`PathResolver` is the legacy single-policy interface. Resolver cache owners must
invalidate filesystem/configuration changes through `mark_dirty_root`.
Structural resolution adds importer, request and mode context without flattening
the legacy resolver's underlying error chain.

Run `cargo test -p import_resolver`; filesystem tests use managed directories.
Default extension probing also accepts `.jsonc`, after the existing extensions;
explicit `.jsonc` requests resolve normally. Package metadata remains strict JSON.
Explicit `.mjsx`/`.cjsx` requests also probe `.mtsx`/`.ctsx` sources respectively,
just as `.mjs`/`.cjs` probe `.mts`/`.cts`; default extension order is unchanged.
`NodeModulesResolverOptions::prefer_js_extension` preserves the legacy `.js`
fast path by default. Set it to `false` to probe exact files followed strictly
by `extensions` in order, including compound suffixes such as `web.js`.
An empty list then disables suffix probing, not exact files or explicit
TypeScript substitutions. Both the list and fast-path policy key package targets.

`structural::StructuralResolver` implements `DependencyResolver` with explicit
import/require conditions and separate caches. Package-target cache keys include
the active export-condition set. Only exact declared externals are accepted;
undeclared builtins fail. External export names are delegated to the host, not
validated as local files. Options retain explicit canonical/preserved symlink
policy. Graph source identities must use the same policy.
Node defaults activate `node`, `node-addons` and `module-sync` in addition to the
request mode; browser defaults activate `browser` and omit those Node conditions.
Structural resolution applies `NodeModulesResolverOptions::alias` as exact,
one-step request replacements before builtin/external policy. For caller-provided
browser stubs, map `fs` and `node:fs` separately to a concrete local CJS/ESM file;
relative paths are importer-relative, absolute paths are also supported.
There is no prefix matching or recursive alias expansion (`fs` does not match
`fs/promises`). Replacements undergo ordinary file resolution, and graph consumers
still enforce their checked source inventory. Missing replacements fail with the
original importer/request. Structural browser resolution rejects unreplaced or
replacement bare/`node:` builtins even when external, and native `.node` addons;
it never injects polyfills or empty modules. Package declaration
order, including an early `default`, determines which active condition wins.
Nested export conditions are evaluated in declaration order at each level,
including `import`/`require` branches containing `types` and runtime `default`
targets. Inactive nested branches may fall through to later parent conditions;
private targets do not.
An exports map is authoritative: unmatched subpaths or inactive conditions fail
resolution, even when a matching physical file exists. Packages without exports
retain legacy filesystem resolution. Browser resolution applies a dependency's
object mapping to its selected legacy package entry (for example, `main` pointing
to `server.mjs` mapped to `browser.mjs`), while preserving importer request/path
mappings. Cached cwd-relative browser destinations are restored to absolute
package-entry paths before normal filename resolution. Node ignores these
browser mappings. Boolean `false` mappings do not
currently synthesize empty modules.
Bare-string exports intentionally expose
the selected root plus arbitrary files within that package, equivalent to adding
`"./*": "./*"`; unlike explicit maps, this shorthand is not Node-encapsulated.

Resolvers never discover, parse or validate `tsconfig.json`. Compiler
`customConditions`/`customSourceConditions`, `extends`, `references`, `baseUrl`
and `paths` have no effect on runtime resolution. Supply additional conditions
explicitly through `NodeModulesResolverOptions::export_conditions` (including
when constructing `MonorepoResolver` or setting `StructuralResolver` policy):

```rust
use import_resolver::{node_resolver::caching::NodeModulesResolverOptions, resolve::TargetEnv};
let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
options.export_conditions.push("source".into());
```

This replaces the removed `read_custom_conditions` API and tsconfig cache.
Use package exports or relative requests instead of compiler path aliases;
TypeScript precompilation remains external. Existing package resolution and
linked-package behavior retain their separate policies; no new symlink support
is implied.

The fence-checker's separate `manual_resolver::resolve_import` classifies relative
and bare requests without compiler aliases. `resolve_ts_import` and the
`tsconfig_paths` crate have been removed, along with the fence API's `project` and
`base_url` options. Bare fence imports are dependency names, not local path aliases.

Call `invalidate` after filesystem/package edits, additions or removals;
`set_policy` invalidates automatically. Invalidation intentionally replaces both
cache trees, including misses/package ancestry. `revision` tracks these policy/
filesystem epochs. This correctness-first API does not promise minimal relinking.

## Scoped filesystem proofs

`StructuralResolver::resolve_dependency_with_inputs` returns a `TrackedDependency`
with the unchanged resolution and an ordered `InputProof`. It uses the same
resolver and intra-build caches as `DependencyResolver`, not a second resolver.
Call `inputs.validate()` before trusting reuse or publishing success; capture
alone is not fresh validation, and old cached results intentionally retain their
original observations. Invalidate and resolve again after validation fails.
Request mode, resolver policy and its revision remain the caller's cache key.

Proofs include consulted positive/negative file candidates, ancestor
`node_modules` and browser package-scope misses, exact manifest byte seals,
canonical/file identities and intermediate symlink/junction targets. Cached
package metadata, browser rewrites and successful/failed package-target lookups
replay the observations that produced them, even if first populated by a legacy
call. Source contents are not hashed for resolution. No directories are
enumerated, and validation bypasses all context/target caches. Tracked calls
surface probe IO errors even where legacy boolean/fallback resolution masks
them. Failed requests are not exposed as reusable persisted records.

Each resolver epoch shares a single-flight `InputSnapshot`, including negative
candidates and manifest bytes. Import/require caches share parsed manifests but
retain separate target results; target keys also include extensions, environment
and symlink policy. Per-target initialization never holds a DashMap shard lock.
Cached wrapping uses the original canonical observation, not another filesystem
lookup. Relative probes also retain the epoch's observations: call `invalidate`
after edits before resolving again. `set_snapshot` replaces both cache trees and
advances the revision when embedding resolution in a shared build epoch.
