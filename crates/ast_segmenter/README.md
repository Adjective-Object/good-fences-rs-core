# ast-segmenter

Extracts owned top-level statement segments from OXC-parsed JavaScript and
TypeScript. Each segment contains module imports/exports, variable declarations
and references, and its ownership span; the parser arena can then be reset.
Spans are zero-based UTF-8 byte ranges with an exclusive end.
Top-level multi-declarator `const`/`let`/`var` declarations, including named
exports, have one segment per declarator. Destructuring stays indivisible;
nested declarations and opaque CommonJS bodies are not split. Segment ownership
ranges are disjoint. Each `facts.emission.segments[i].source` is a `MultiSpan`
(an inline `SmallVec` bounded to four original-source pieces): split segments
share the declaration/export prefix and copy only their own declarator.
Materialization adds a statement terminator; all occurrence and rewrite offsets
remain in the original source, with no transformed-source reparse.
`module_parse_count()` reports source-module parser invocations on the current
thread, distinguishing extraction from accidental re-extraction.

Read persisted records through borrowed accessors, for example
`parsed.facts().emission().segments()[i].source()`. Strings and collections
borrow their storage; optional records return `Option<&T>`. Construction and
finalization stay inside extraction, before provenance is sealed. `into_parts`
and `into_facts` transfer ownership only at owned-adapter boundaries.
This API encapsulation does not change Serde/CBOR layout or introduce a
byte-backed representation.

`facts.emission.execution` owns preorder callable/instance-initializer contexts,
their parent links and reference/invocation context indices. Callable spans include
parameter defaults; computed keys and static class initialization inherit their
surrounding context. Only direct synchronous non-generator literal calls carry an
immediate activation link. Named/aliased calls and callbacks remain unproven.
Placement eager reads/writes use this classification; deferred writes remain in
the complete `writes` collection for availability/relocation checks. These are
syntax-local facts, not purity, a call solver or a linked execution schedule.
Effect timing resolves the exact owning context recorded in `EffectSite.deferred`,
not containment of its operand span; callable heritage/key operands do not defer
the surrounding class operation. Missing recorded contexts are errors.

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

`parse_module_with_declarations` accepts explicit effective Ecco `LocalFacts`;
the default entrypoint uses `DeclarationConfig::default().effective()`.
For repeated extraction, construct `PreparedDeclarations::from_config` (or
`new(LocalFacts)`) once and use `parse_module_with_prepared_declarations`.
It validates and owns immutable facts together with their typed 128-bit XXH3
`DeclarationFingerprint`; cloning shares the facts and never rehashes them.
`fingerprint::declaration_fingerprint_count()` counts declaration serialization/
hash operations on the current thread, including compatibility wrappers.
The declaration fingerprint replaces the former 32-byte SHA-256 declaration
field; source and sealed extraction integrity hashes remain SHA-256.
ESM `facts.ecco` contains owned lazy roots/functions extracted from the same
checked AST, at most one root per actual segment (including split declarators).
CommonJS keeps `None` and remains opaque. `facts.ecco_declarations_hash` seals
canonical effective contracts into provenance for both formats. No analyzed effects or
graph resolutions are cached. PURE hints remain policy-sensitive: consumers
disabling annotation trust must conservatively retain annotated invocations.

`ParsedModule::provenance()` binds the exact source bytes, effective
`ParserConfig` and owned body/ranges/facts to extraction. `from_provenance`
reconstitutes matching cache records without parsing; it rejects re-paired text,
changed records and parser modes. These integrity hashes prevent ordinary API
misuse, not deliberate forgery by a cache publisher.

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
borrows escape. Relative template/concatenation dependencies store literal
pieces separated by runtime gaps; unconstrained nonliteral dependencies fail.
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
| Global CJS `require()` | Exactly one literal or relative template/concatenation argument; parenthesized expressions accepted; shadowed calls are ordinary code |
| Relative template/concatenation import/require | One real occurrence with owned literal pieces, a leading `./` or `../`, and the original argument span; graph capture is a later step |
| Unconstrained/bare nonliteral import/require, extra/spread require arguments, optional require call | Diagnostic, never a missing dependency |
| Require reassignment, escaped/aliased loader, `require.resolve`, `module.require`, unknown module access | Diagnostic; only direct loader calls and `module.exports` member reads are supported |
| CJS `require`/`module`/`exports` declarations at module scope | Diagnostic (wrapper parameters and var/function redeclarations must not be misidentified as independent locals); nested shadows and pure type-space declarations supported |
| CJS `__filename`/`__dirname` declarations | Top-level var/function redeclarations reuse wrapper parameters; lexical redeclarations are diagnostics; nested shadows supported |
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

## Cached emission recipes

`facts.emission` exposes owned DTOs in `ast_segmenter::emission`, indexed by the
existing statement segments. Each `SegmentRecipe` records `connector_only`,
declaration/hoisted availability, forward uses, eager-read and write reference
indices, conservative function/class/arrow name restrictions and namespace uses.
Namespace direct-read candidates are distinct from receiver, tag, mutation,
delete and escaping uses. Classification covers every import binding, including
named/default imports that may resolve to namespace reexports; consumers select
these alternatives only for linked namespace identities. Whether an export can
replace a candidate stays linked.
Nested-binding IDs and free-reference indices support conservative capture checks.

Sparse `OperationKind` slots address existing binding/declaration, reference,
dependency and invocation IDs/indices, or original wrapper/comment spans.
They describe shorthand expansion (including object-literal `__proto__`),
import/export removal, default expression initialization/anonymous function
hoisting, namespace alternatives and in-segment source-hint removal.
`OperationKind::span` resolves occurrence ranges on validated facts.
Operations are preorder with explicit parents; a replacing parent owns children.
`SourceSlot` records already-composed operation roots for default values, PURE
callees/arguments and chain subexpressions. Consumers need not rediscover edit
containment or parse generated/retained text to recover lexical identity.

PURE alternatives contain a source base and inner-to-outer member/call steps,
original argument/spread facts, guard boundaries and an await-crossing veto.
Parentheses terminate outer chains; unsupported nested optional-call/private
chains have no lowering alternative. Await checks use the invocation's own
function scope, not awaits in nested function bodies. These recipes never
declare a call pure or choose names, resolved requests, retention or ownership.
CJS keeps one opaque recipe with dependency/hint slots only. Comments outside
all statement spans deliberately remain outside source-hint cleanup.
