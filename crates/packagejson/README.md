# packagejson

Reads the package metadata used by the cached resolver: package names, entry
fields, browser mappings and exports. Conditional export entries retain JSON
declaration order; consumers must select the first active condition, including
`default`, rather than prioritizing their own condition list.
Boolean `main` values (including the `main: false` used by subpath-only
packages) are absent legacy entry paths, not filenames or disabled modules.
String paths remain unchanged; exports still govern package access.
Consumers use `package_relative_path` at metadata file-path boundaries: rooted
paths lose their filesystem root/native volume and resolve from the manifest's
directory (for example, `/js/Abt.js` becomes `./js/Abt.js`). This intentional
Node deviation does not rewrite parsed metadata, relative paths, bare dependency
redirects or direct user import requests. Dot segments remain available to each
field's existing target validator; rebasing does not authorize package escapes.
Nested condition objects preserve order at every level. Root conditional
shorthand normalizes to an exact `"."` export, without adding wildcard access.
Fallback arrays retain ordered recursive targets, including condition objects
and nested arrays. Root array shorthand also exports only `"."`. Invalid scalar
array entries remain distinct from `null`, so resolution can skip invalid targets
without losing private/fallback semantics.
Bare-string `exports` normalizes to a `"."` entry plus `"./*": "./*"`, exposing
arbitrary files within that package. This is an intentional bundler extension
to Node's root-only shorthand. Explicit export maps remain encapsulated;
unmatched or private subpaths do not gain a filesystem fallback.

`imports` retains an explicit `#`-keyed map of the same recursive ordered target
trees (conditions, arrays, strings and null). It never receives exports'
bare-string wildcard extension; the resolver validates import keys and targets.

Run `cargo test -p packagejson`.

Filesystem reads parse raw buffered bytes with `serde_json::from_slice`, never
byte-at-a-time `from_reader(File)`. The context-cache provenance hook seals
exactly those parsed bytes and records original path/link identity, including
missing manifests. Cached consumers must retain that seal, not hash a newer
manifest while reusing older parsed metadata.

Snapshot-aware observers reuse exact sealed manifest bytes within one filesystem
epoch. Context readers consume those bytes directly; they do not reopen or
recapture a manifest on each request.
