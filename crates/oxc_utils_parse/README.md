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
Static ESM syntax in a CommonJS-mode `.js` file reports the missing required
package `"type":"module"` (or explicit CommonJS configuration), suggesting that
declaration or `.mjs` instead of `.js`. This is a diagnostic only: it does not
retry parsing with a different format or silently infer ESM.

The tracker adapter is unchanged: it accepts TS syntax in `.js` and unknown
extensions, uses OXC's content-based module/script detection, and does not
interpret `.cjs` as CommonJS. Neither API skips generated-file marker comments.

The bundler consumer in `ast-segmenter::parse_module` applies a stricter,
documented source-local support gate after semantic analysis, including opaque
CommonJS extraction. Parser acceptance alone is not support for JSX/TS runtime
transforms, mixed modules, top-level await, attributes or eval.

Run `cargo test -p oxc_utils_parse` for parser and comment-attachment tests.
