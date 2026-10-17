# ast-segmenter

Extracts owned top-level statement segments from OXC-parsed JavaScript and
TypeScript. Each segment contains module imports/exports, variable declarations
and references, and its source span; the parser arena can then be reset.
Spans are zero-based UTF-8 byte ranges with an exclusive end.

Variable scopes retain whether an escaped name is referenced only in type
positions. A name referenced in both type and value positions remains a runtime
dependency. Consumers can inspect all names or their type-only classification.

The compatibility `SegmentGraph` follows resolved imports and re-exports by
export name. Side-effect imports and opaque requires depend on every target
statement, including files without exports; they do not model evaluation timing.

`parse_module(logger, source, path, package_format)` is the fallible bundler
entrypoint. It checks extension-sensitive parser and semantic diagnostics before
extracting owned records. Generated-file markers do not skip source, and
rejected statements fail the entire operation. `try_segment_file` is available
for callers that already validated an AST and semantic model; `segment_file`
retains the unused tracker's legacy behavior.

`ParsedModule::facts` contains the owned occurrence IR in `owned`: bindings,
declaration and reference ranges, import aliases, export mappings, directives,
hashbang and individual static/dynamic/require dependencies. Binding and
dependency IDs are source byte offsets (first declaration and dependency site,
respectively), meaningful only within the exact source and parse policy.
Semantic resolution links forward references and nested/shadowed bindings;
unresolved globals have no binding ID. Repeated specifiers are not coalesced.
Reference read/write and type-only flags survive independently of legacy name
sets. Collections have deterministic source ordering; no OXC IDs or arena
borrows escape. Nonliteral dependencies fail instead of disappearing.
`function_scoped` includes both `var` and parameters, as in OXC's binding flags;
the occurrence IR does not infer evaluation timing or TDZ safety.

These are parse records, not emitted JavaScript: accepting TS/JSX does not
implement their transforms. CommonJS parsing is available in `oxc_utils_parse`,
but `parse_module` explicitly rejects CommonJS extraction pending an opaque-body
record. TS import/export assignments and namespace exports are also rejected.
No source-file IO, cache, linking, effect analysis or emission is added here.

Run `cargo test -p ast-segmenter` for extraction and scope regressions.
