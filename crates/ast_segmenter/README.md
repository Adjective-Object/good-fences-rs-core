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
rejected statements fail the entire operation. `try_segment_file` checks legacy
statement extraction for callers that already validated an AST and semantic
model; it does not perform the full bundler support-policy/occurrence checks.
`segment_file` retains the unused tracker's legacy behavior.

`ParsedModule::facts` contains the owned occurrence IR in `owned`: effective
`commonjs` parser-format bit (including package policy for `.js`), bindings,
declaration and reference ranges, import aliases, export mappings, directives,
hashbang and individual static/dynamic/require dependencies. Binding and
dependency IDs are source byte offsets (first declaration and dependency site,
respectively), meaningful only within the exact source and parse policy.
Semantic resolution links forward references and nested/shadowed bindings;
unresolved globals have no binding ID. Repeated specifiers are not coalesced.
Reference read/write and type-only flags, ambient declarations and merged
type/value bindings survive independently of legacy name sets. Collections
have deterministic source ordering; no OXC IDs or arena
borrows escape. Nonliteral dependencies fail instead of disappearing.
`function_scoped` includes both `var` and parameters, as in OXC's binding flags;
the occurrence IR does not infer evaluation timing or TDZ safety.

`ParsedModule::body` distinguishes ESM from CommonJS. CommonJS has exactly one
opaque `Segment`, including directives, comments and whitespace, excluding the
hashbang. Its internals must not be independently retained or tree-shaken.
Dependencies remain individual occurrences in `facts`; each has its nearest
lexically enclosing function span, if any. This is not an eagerness flag:
conditions, class fields, unreachable code and function calls retain their
original execution timing inside the opaque body. The segment's legacy
dependency sets are only a conservative projection.

`Reference::commonjs` classifies unresolved `require`, `module` and `exports`
wrapper references; lexical shadows are ordinary bindings. `module.exports`
replacement, `exports` alias replacement and `module` assignment remain in the
body verbatim. No static list of CommonJS named exports is guessed.

## Source-local support matrix

These are **parse records**, not emitted JavaScript or runtime conformance.
Low-level `oxc_utils_parse` accepts broader syntax; this bundler gate deliberately
rejects unsupported constructs with path and source ranges. Legacy adapters are
unchanged.

| Construct | `parse_module` policy |
| --- | --- |
| ESM imports/exports, aliases, defaults, string names, namespace/star re-exports | Owned mappings and dependency sites; export linking is a later gate |
| Literal dynamic `import()` | One dependency per site; loading belongs to bundler |
| Literal global CJS `require()` | Exactly one string-literal argument; parenthesized expressions accepted; shadowed calls are ordinary code |
| Nonliteral import/require, extra/spread require arguments, optional require call | Diagnostic, never a missing dependency |
| Require reassignment, escaped/aliased loader, `require.resolve`, `module.require`, unknown module access | Diagnostic; only direct literal loader calls and `module.exports` member reads are supported |
| CJS runtime wrapper-name declarations at module scope | Diagnostic (wrapper parameters and var/function redeclarations must not be misidentified as independent locals); nested shadows and pure type-space declarations supported |
| CJS `module`/`exports` reassignment | Preserved verbatim with reference write flags; no alias or named-export inference |
| CJS directives/hashbang/top-level return | Preserved; hashbang recorded separately; one opaque body even when empty |
| CJS wrapper `arguments` access, including arrows capturing it | Diagnostic; ordinary nested functions' own `arguments` supported |
| Mixed-file static ESM/CJS syntax or unshadowed ESM references to CJS wrapper names | Diagnostic; explicit extension/package format is never guessed from content |
| Unshadowed eval, nested `with`, `import.meta` | Diagnostic; lexically shadowed eval is an ordinary call |
| Top-level await/for-await | ESM source spans in `facts.top_level_await`, excluding nested functions; CJS diagnostic. Dependency/SCC scheduling belongs to the native bundler host |
| Import attributes/options/phases (including type import options) | Diagnostic |
| Erasable TS annotations/types/assertions, TS import types | Owned syntax facts and type-only references/dependencies; **not erased or executable output** |
| JSX, decorators, auto-accessors, TS enums/namespaces/parameter properties, TS import/export assignments and namespace exports, resource management | Unsupported-transform diagnostic |
| Native-addon/asset specifiers, external/builtin specifiers, require-of-ESM targets | Raw target string and occurrence preserved, **not resolved, externalized, transformed or validated for execution**; these need resolver/linker policy, including require-of-async-ESM diagnostics |

Owned occurrence records and `ModuleBody` support serde for the versioned
`segment_cache` codec. Span serialization is a pair of byte offsets, not OXC
arena identity. The codec validates these offsets against the exact source.
Legacy statement dependency/name sets are deliberately not serialized.

`facts.effects` records conservative, deterministic `EffectSite` reasons and
byte ranges. `deferred = None` means module evaluation; a function/instance-field
range defers execution (including default arguments). Ordinary function/getter
creation is not invocation. Unknown syntax, reads that may throw, getters/proxies,
coercions, spreads/destructuring, class heritage/computed keys/static blocks and
opaque CJS remain conservative. This is syntax-local evidence, not a claim that
an effect always executes or an effect-policy-derived purity result.

There is no source-file IO, cache, linking, user effects policy or emission here.
Target-dependent checks cannot be decided from source syntax alone.
The separate `bundler` Node backend now executes the supported synchronous
JavaScript ESM/CJS subset, including opaque CJS cycles, requested-name proxies,
literal dynamic imports and native ESM TLA/for-await scheduling.
Its explicit namespace/snapshot policy is documented in that crate's README.
Require-of-async-ESM, externals/assets, TS/JSX lowering and source maps remain gates.

`facts.invocations` owns callee/receiver/argument ranges, spread iteration,
optional-chain boundaries, lexical callee identity, syntactic result-use context
and exact PURE comment sites. Only exact block-comment `@__PURE__`/`#__PURE__`
markers with pinned-OXC call/new attachment qualify (including parentheses and
contiguous trivia comments); declaration-leading comments, strings, trailing or
prose substrings do not. These are hints, never unconditional purity facts.
Function/instance-field records remain deferred. CommonJS has no internal
invocation rewrite records because its body is opaque.
Member shape normalizes parenthesized optional-chain references for structural
matching without erasing the distinct original short-circuit boundary. Concise
arrow expression bodies have required return values, not discarded statement
results, even though OXC represents them internally as expression statements.

Run `cargo test -p ast-segmenter` for extraction and scope regressions.
