# packagejson

Reads the package metadata used by the cached resolver: package names, entry
fields, browser mappings and exports. Conditional export entries retain JSON
declaration order; consumers must select the first active condition, including
`default`, rather than prioritizing their own condition list.
Bare-string `exports` normalizes to the equivalent `"."` entry. As in Node,
that exposes only the root; use an explicit `"./*": "./*"` export to expose
arbitrary package files without weakening private-subpath checks.

Run `cargo test -p packagejson`.
