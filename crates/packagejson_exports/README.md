# packagejson_exports

Derives package-relative export rewrites and reverse export visibility from
`packagejson::PackageJsonExports`. Exact exports precede patterns. Star patterns
use Node's specificity rules: longest prefix before `*`, then longest full key,
independent of map iteration order or the requested conditions.
Conditions are an active set, not a priority list: the selected subpath's first
active condition in package declaration order wins, with `default` always active.
Nested objects apply the same selection recursively. An active branch with no
matching descendant resumes its parent's remaining conditions; an explicit
private leaf still blocks fallback. Wildcard substitution applies only after
selecting the final leaf. Reverse visibility includes nested target paths and
reports their outer condition names.
Fallback arrays recursively try targets in declaration order. Skip inactive
condition branches and invalid package targets; retain the last null/invalid
result if no path is selected. Empty arrays are private. Null blocks later
conditions, but inside arrays it allows later candidates, matching Node 22.
Invalid condition-object configurations are errors, not skippable targets.
Selection precedes filesystem probing: a selected missing file never retries
later array entries. Nested arrays also participate in reverse visibility.
The package reader normalizes bare-string exports to `"."` plus `"./*": "./*"`,
intentionally exposing arbitrary package files rather than Node's root-only
shorthand. Explicit maps retain their declared visibility; more-specific private
patterns still block access. Rewritten request subpaths cannot escape the package
root through parent traversal.

`rewrite_package_import` shares the recursive condition/array evaluator but uses
imports-specific validation and matching: exact keys, then single-star patterns
by longest prefix/full key, with no legacy directory-prefix access. Substitute
every target star and reject traversal, encoded prohibited segments and URL
targets. Rooted metadata file targets intentionally rebase to `./` paths through
`packagejson::package_relative_path` before the existing target validation,
recursively for exports/imports conditions and arrays. Native roots/volumes are
not filesystem authority; invalid rebased targets remain invalid. Derived export
rewrites also use rebased paths for reverse visibility.
Imports may redirect to bare dependencies or other `#`
requests; resolving those requests and bounding cycles belongs to the resolver.

Run `cargo test -p packagejson_exports` for rewrite and visibility regressions.
