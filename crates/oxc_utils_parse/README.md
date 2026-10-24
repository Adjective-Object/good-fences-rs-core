# oxc_utils_parse

Parser helpers for OXC 0.155.0, requiring Rust 1.97 or newer. `parse_file` preserves
the unused tracker's compatibility policy: parse every extension as TypeScript,
with JSX enabled for `.jsx` and `.tsx`. `parse_ts` and `parse_tsx` select those
language modes directly. Callers must check parser and semantic diagnostics.

`parse_bundler_file` is the separate, fallible bundler policy. It infers JS,
JSX, TS or TSX from a supported extension and requires an explicit package
`ModuleFormat`. `.mjs`/`.mts`/`.mjsx`/`.mtsx` force ESM and
`.cjs`/`.cts`/`.cjsx`/`.ctsx` force CommonJS. `bundler_source_type` shares these
language/extension rules with cache configuration. Ambiguous `.js`/`.jsx`/`.ts`/
`.tsx` inputs use the supplied package format as a fallback. Static import/export
syntax or `import.meta` selects ESM under a CommonJS fallback; dynamic `import()`
alone does not. Detected modules are reparsed under strict ESM grammar, preserving
CommonJS wrapper returns only for actual CommonJS inputs. It rejects
unknown extensions, parser diagnostics and parser aborts, with path and OXC
diagnostic ranges. Semantic diagnostics still need checking before extraction.
Package discovery belongs to the caller; this helper does no filesystem IO.
`parse_with_source_type` is the explicit-grammar boundary used by bundler parsing
and output validation. Upstream parser unwinds become errors containing the
filename, grammar and panic payload; the process-wide panic hook is unchanged.
This improves diagnostics, not parser correctness or recovery from process aborts.
The pinned revision includes upstream's PURE member-access fix: annotations on
calls followed by property access remain applied inside nested assignments,
including the MobX iterator-prototype initializer. Sources and annotations are
not rewritten to work around parser bugs.
Explicit `.cjs`/`.cjsx` files containing ESM syntax fail instead of being
reclassified. TypeScript CommonJS inputs retain their explicit mode for downstream
type erasure and the unsupported ESM-to-CommonJS transformation gate.

The tracker adapter is unchanged: it accepts TS syntax in `.js` and unknown
extensions, uses OXC's content-based module/script detection, and does not
interpret `.cjs` as CommonJS. Neither API skips generated-file marker comments.

The bundler consumer in `ast-segmenter::parse_module` applies a stricter,
documented source-local support gate after semantic analysis, including opaque
CommonJS extraction. Parser acceptance alone is not support for JSX/TS runtime
transforms, mixed modules, top-level await, attributes or eval.

Run `cargo test -p oxc_utils_parse` for parser and comment-attachment tests.
