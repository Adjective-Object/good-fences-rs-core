# packagejson

Reads the package metadata used by the cached resolver: package names, entry
fields, browser mappings and exports. Conditional export entries retain JSON
declaration order; consumers must select the first active condition, including
`default`, rather than prioritizing their own condition list.

Run `cargo test -p packagejson`.
