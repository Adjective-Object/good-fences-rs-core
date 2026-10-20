# import_resolver

Resolves Node/package requests using the repository's cached resolver.
`PathResolver` is the legacy single-policy interface. Resolver cache owners must
invalidate filesystem/configuration changes through `mark_dirty_root`.

Run `cargo test -p import_resolver`; filesystem tests use managed directories.
Default extension probing also accepts `.jsonc`, after the existing extensions;
explicit `.jsonc` requests resolve normally. Package metadata remains strict JSON.

`structural::StructuralResolver` implements `DependencyResolver` with explicit
import/require conditions and separate caches. Package-target cache keys include
the active export-condition set. Only exact declared externals are accepted;
undeclared builtins fail. External export names are delegated to the host, not
validated as local files. Options retain explicit canonical/preserved symlink
policy. Graph source identities must use the same policy.
Node defaults activate `node`, `node-addons` and `module-sync` in addition to the
request mode; browser defaults omit those Node conditions. Package declaration
order, including an early `default`, determines which active condition wins.
An exports map is authoritative: unmatched subpaths or inactive conditions fail
resolution, even when a matching physical file exists. Packages without exports
retain legacy filesystem resolution. Bare-string exports intentionally expose
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
