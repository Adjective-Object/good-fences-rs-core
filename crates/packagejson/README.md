# packagejson

Reads the package metadata used by the cached resolver: package names, entry
fields, browser mappings and exports. Conditional export entries retain JSON
declaration order; consumers must select the first active condition, including
`default`, rather than prioritizing their own condition list.
Bare-string `exports` normalizes to a `"."` entry plus `"./*": "./*"`, exposing
arbitrary files within that package. This is an intentional bundler extension
to Node's root-only shorthand. Explicit export maps remain encapsulated;
unmatched or private subpaths do not gain a filesystem fallback.

Run `cargo test -p packagejson`.

Filesystem reads parse raw buffered bytes with `serde_json::from_slice`, never
byte-at-a-time `from_reader(File)`. The context-cache provenance hook seals
exactly those parsed bytes and records original path/link identity, including
missing manifests. Cached consumers must retain that seal, not hash a newer
manifest while reusing older parsed metadata.
