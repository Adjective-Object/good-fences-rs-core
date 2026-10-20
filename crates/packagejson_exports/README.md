# packagejson_exports

Derives package-relative export rewrites and reverse export visibility from
`packagejson::PackageJsonExports`. Exact exports precede patterns. Star patterns
use Node's specificity rules: longest prefix before `*`, then longest full key,
independent of map iteration order or the requested conditions.
Conditions are an active set, not a priority list: the selected subpath's first
active condition in package declaration order wins, with `default` always active.
The package reader normalizes bare-string exports to `"."` plus `"./*": "./*"`,
intentionally exposing arbitrary package files rather than Node's root-only
shorthand. Explicit maps retain their declared visibility; more-specific private
patterns still block access. Rewritten request subpaths cannot escape the package
root through parent traversal.

Run `cargo test -p packagejson_exports` for rewrite and visibility regressions.
