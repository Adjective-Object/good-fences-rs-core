# import_resolver

Resolves Node/package requests using the repository's cached resolver.
`PathResolver` is the legacy single-policy interface. Resolver cache owners must
invalidate filesystem/configuration changes through `mark_dirty_root`.

Run `cargo test -p import_resolver`; filesystem tests use managed directories.

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

The nearest bounded `tsconfig.json` adds `compilerOptions.customConditions`
to the active export conditions. `customSourceConditions` is a compatibility
alias; specifying both names is an error. Conditions retain package declaration
priority. `baseUrl` and `paths` are ignored, never runtime aliases or local-file
fallbacks. Use package exports instead. Configs must be strict JSON with direct
conditions; `extends`/`references` remain explicit unsupported errors.
The separate legacy `manual_resolver` used by fence checks is unchanged.

Call `invalidate` after filesystem/package/tsconfig edits, additions or removals;
`set_policy` invalidates automatically. Invalidation intentionally replaces both
cache trees, including misses/config ancestry. `revision` tracks these policy/
filesystem epochs. This correctness-first API does not promise minimal relinking.
