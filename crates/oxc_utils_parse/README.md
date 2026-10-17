# oxc_utils_parse

Parser helpers for the workspace's pinned OXC revision. `parse_file` preserves
the unused tracker's compatibility policy: parse every extension as TypeScript,
with JSX enabled for `.jsx` and `.tsx`. `parse_ts` and `parse_tsx` select those
language modes directly. Callers must check parser and semantic diagnostics.

`parse_bundler_file` is the separate, fallible bundler policy. It infers JS,
JSX, TS or TSX from a supported extension and requires an explicit package
`ModuleFormat`. `.mjs`/`.mts` force ESM and `.cjs`/`.cts` force CommonJS; other
extensions use the supplied package format, never content detection. It rejects
unknown extensions, parser diagnostics and parser aborts, with path and OXC
diagnostic ranges. Semantic diagnostics still need checking before extraction.
Package discovery belongs to the caller; this helper does no filesystem IO.

The tracker adapter is unchanged: it accepts TS syntax in `.js` and unknown
extensions, uses OXC's content-based module/script detection, and does not
interpret `.cjs` as CommonJS. Neither API skips generated-file marker comments.

Run `cargo test -p oxc_utils_parse` for parser and comment-attachment tests.
