# ast-segmenter

Extracts owned top-level statement segments from OXC-parsed JavaScript and
TypeScript. Each segment contains module imports/exports, variable declarations
and references, and its ownership span; the parser arena can then be reset.
Spans are zero-based UTF-8 byte ranges with an exclusive end.
Computed dynamic imports retain bounded relative patterns, or a two-empty-piece
pattern for recursive importing-directory capture when no relative prefix exists.
Attached `/* webpackIgnore: true */` and line comments leave imports native:
no dependency is extracted, but argument references and effects remain.
JSDoc, strings and unrelated comments are not magic annotations; explicit false
uses normal dependency extraction. Comment values are parsed, never executed.
Callers supplying a semantic model to `segment_file` must enable
`SemanticBuilder::with_build_nodes(true)` for reference spans.
Module requests containing lone UTF-16 surrogates are rejected rather than
lossily resolved. Such computed property names retain unknown property metadata;
their original source is preserved.
Line-comment annotation matching is ASCII-case-insensitive and tolerates
arbitrary Unicode comment text without slicing across UTF-8 boundaries.
Top-level multi-declarator `const`/`let`/`var` declarations, including named
exports, have one segment per declarator. Destructuring stays indivisible;
nested declarations and opaque CommonJS bodies are not split. Segment ownership
ranges are disjoint. Each `facts.emission.segments[i].source` is a `MultiSpan`
(an inline `SmallVec` bounded to four original-source pieces): split segments
share the declaration/export prefix and copy only their own declarator.
Materialization adds a statement terminator; all occurrence and rewrite offsets
remain in the original source, with no transformed-source reparse.
Recipes also retain syntax-only deferred reconstruction alternatives: named root
functions, simple identifier variables with composed original initializer slots,
effect statements, or unsupported syntax. Scheduling/eligibility and chosen
names remain linker decisions, never cached policy.
`facts.emission.dynamic_namespaces` records conservative AST observations for
each dynamic import: supported static property reads/call receivers, simple
destructuring or discarded results, versus escaping/reflective/writable uses.
Alias references are checked by binding identity, including nested uses.
`(await import('./x')).name` and `import('./x').then(ns => ns.name)` retain
source-local property observations, including parenthesized forms and receiver
calls. Repeated imports stay separate; property/receiver pairs within each
occurrence are sorted and unique, not a combined export-demand root.
Parameter writes/redeclarations, namespace escapes and reflection fall back
conservatively. Concise-arrow results are returned, not discarded;
`await import('./x').name` reads the promise, not the module namespace.
`uses_this` records source syntax for conservative receiver eligibility.
Target export surfaces, retention and placement remain linker decisions.
`module_parse_count()` reports source-module parser invocations on the current
thread, distinguishing extraction from accidental re-extraction.

`facts().emission().lazy_modules()` and `lazy_selectors()` expose syntax-only
construction candidates through borrowed owned-or-wire sequences. Module candidates
retain the owning segment, declared binding, constructor reference index, literal
callback import occurrence, declarator/construction spans and all original
argument slots. `import_span()` identifies the directly returned import, not
imports nested in options. `callback()` and `options()` borrow the first/second
slot without copying expressions. Selector candidates retain constructor/module/
getter references, module binding, selected property/token span, construction
span and every original argument. Optional `result_binding()` and
`result_declaration()` identify the actual directly initialized identifier and
complete declarator, never a nearby local inferred from source positions.
Annotated TS declarators omit this optional boundary evidence.
`module_argument()` and `getter()` borrow their original slots. Slot operations
are composed against the owning segment recipe, including nested replacements.
`property_span()` anchors the decoded property to its original identifier token,
including Unicode escapes, without reparsing on cache hits.

Constructors need only be identifier references: imported aliases, lexical shadows
and unknown names all remain candidates. `LazyWrapperKind` is an imported/local
spelling hint (`Unknown`, `LazyImport`, `LazyAction`), never runtime proof. Currently
callbacks are non-async zero-parameter concise arrows returning a literal import;
getters are non-async one-identifier-parameter concise arrows returning that
binding's non-optional dotted property, without a parameter default. Parentheses
and parameter annotations are preserved; expression casts/assertions are not
supported candidates. Unsupported getters are omitted, but all ordinary
references remain for later all-reference
certification. Extra arguments, spreads, options effects and initializer syntax
are preserved, not evaluated, copied or certified. No promise rewrite, runtime
recognition, target eligibility, graph obligations or emission plan is extracted.

Optional `options_properties()` evidence is absent for values containing JSX,
TypeScript types/generics, regexps or division, which the borrowed boundary proof
does not support. This only disables later options eligibility: original
candidates, references, dependencies and argument slots remain intact. JSX still
requires transformation before the public bundler parse-policy gate accepts it.

Read persisted records through borrowed accessors, for example
`parsed.facts().emission().segments().get(i).unwrap().source().spans()`.
`views` provides Copy owned-or-wire tables, value enums and indexed/iterable
`Sequence`s; strings borrow the original storage. Construction and finalization
stay inside extraction, before provenance is sealed. `FactsStorage` retains an
`Arc` of owned facts or a verified `WireRecord`. `ParsedModule::from_wire` retains
that owner without unpacking facts, Ecco precursors or statement ranges.
`into_parts` explicitly constructs graph-owned scalar ranges at the graph boundary;
`into_facts` transfers storage. Owned-only adapters use `owned_facts`,
`owned_segments` or `FactsStorage::into_owned`, never a hidden wire conversion.

`ParsedModule::to_flatbuffer(source, oxc_revision)` checks extraction provenance
before encoding the complete typed module, including Ecco's existing encoder.
The wire seal hashes exact nested-module bytes independently of the outer
checksum. `from_wire` checks source/provenance but is not a semantic validation
gate: persistent callers must also run `segment_cache::validate_parsed`.
Source/configuration/range/fact validation, graph linking, effect solving and native
emission consume these views. Production `segment_cache` loads `.seg.fb` through
this owner and the shared semantic gate. Owned extraction provenance also hashes
the typed module, not CBOR; warm provenance checks reuse the verified immutable
owner and check exact table/range identity without rescanning its tree.
`to_flatbuffer` on a wire module returns its already checked bytes.
`facts.emission().source_map()` borrows the last parser-identified
`sourceMappingURL` comment span, including trailing comments. Only source-local
evidence is sealed; map loading, resolution and output policy belong to bundler.
Serde remains for debug/test projections and the independent once-per-configuration
declaration fingerprint (CBOR for compatibility), never per-file wire loading.

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

`ParsedModule::facts` views the occurrence IR extracted into `owned`: effective
`commonjs` parser-format bit (including package policy for `.js`), bindings,
declaration and reference ranges, import aliases, export mappings, directives,
hashbang and individual static/dynamic/require dependencies. Binding and
dependency IDs are source byte offsets (first declaration and dependency site,
respectively), meaningful only within the exact source and parse policy.
Lazy module candidates optionally carry complete plain-options key/value spans
(`LazyOptionProperty`), read identically through borrowed owned/wire views.
This structural evidence excludes initializer/prototype keys and unsupported
property forms, never proves values pure or constructors trusted. Options remain
one original composed slot; effects/captures must not be duplicated by consumers.
Semantic resolution links forward references and nested/shadowed bindings;
unresolved globals have no binding ID. Repeated specifiers are not coalesced.
Reference read/write and type-only flags, ambient declarations and merged
type/value bindings survive independently of legacy name sets. Collections
have deterministic source ordering; no OXC IDs or arena
borrows escape. Relative template/concatenation dependencies store literal
pieces separated by runtime gaps; unconstrained nonliteral dependencies fail.
`function_scoped` includes both `var` and parameters, as in OXC's binding flags;
the occurrence IR does not infer evaluation timing or TDZ safety.

Each reference's `async_deferred` flag permits introducing suspension in its
nearest executing async function/arrow/method body; it does not mean an await
has already executed. Nested synchronous bodies reset it. Parameters and class
field/static initialization are false, but nested async bodies establish their
own context. Computed keys retain the surrounding evaluation context. This is
cached syntax evidence only; extraction and ordinary emission add no awaits.

`ParsedModule::body` distinguishes ESM from CommonJS. CommonJS has exactly one
opaque `Segment`, including directives, comments and whitespace, excluding the
hashbang. Its internals must not be independently retained or tree-shaken.
Dependencies remain individual occurrences in `facts`; each has its nearest
lexically enclosing function span, if any. This is not an eagerness flag:
conditions, class fields, unreachable code and function calls retain their
original execution timing inside the opaque body. The segment's legacy
dependency sets are only a conservative projection.

`Reference::commonjs` classifies unresolved `require`, `module` and `exports`
wrapper references only in CommonJS; lexical shadows and ESM environment reads
are ordinary bindings/references. `module.exports`
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
| ESM environment probes | `typeof require`, read-only `module`/`exports` references and property reads remain native globals, including lodash-style short-circuit aliases; no injected bindings or constant replacement |
| ESM direct unshadowed literal `require()` | Preserves ESM format and occurrence recipes, including parenthesized calls; target/activation support is a linker check. ESM require contexts remain rejected |
| Mixed-file static ESM/CJS syntax, ESM wrapper writes/deletes/updates/calls/construction/tags, or other ESM loader use except `typeof require` | Diagnostic; direct require support never enables CommonJS wrapper operations |
| Unshadowed eval, nested `with`, unsupported `import.meta` | Diagnostic; lexically shadowed eval is an ordinary call |
| Literal relative `new URL(request, import.meta.url)` with unshadowed URL | Typed URL occurrence and original literal emission slot; target/entry policy belongs to the bundler |
| Top-level await/for-await | ESM source spans in `facts.top_level_await`, excluding nested functions; CJS diagnostic. Dependency/SCC scheduling belongs to the native bundler host |
| Import attributes/options/phases (including type import options) | Diagnostic |
| Erasable TS annotations/types/assertions, TS import types | Owned syntax facts and type-only references/dependencies; **not erased or executable output** |
| JSX, decorators, auto-accessors, TS enums/namespaces/parameter properties, TS import/export assignments and namespace exports, resource management | Unsupported-transform diagnostic |
| Native-addon/asset specifiers, external/builtin specifiers, require-of-ESM targets | Raw target string and occurrence preserved, **not resolved, externalized, transformed or validated for execution**; these need resolver/linker policy, including require-of-async-ESM diagnostics |

Environment reads do not prove a guard true or false: host globals, getters,
short-circuit evaluation, unguarded ReferenceErrors and shadowed lexical TDZ all
retain JavaScript semantics. Aliases are ordinary values, not inferred CommonJS
wrappers or loaders. Direct wrapper operations remain a conservative source-local
gate, even inside a guard. Static ESM syntax disambiguates CommonJS-default
`.js`/`.jsx`/`.ts`/`.tsx`; dynamic import alone does not. Explicit m/c extensions,
including JSX variants, override that fallback. ESM syntax in `.cjs`/`.cjsx`
is an error, not a format override. Provenance seals the effective format and
detection policy so cache hits do not repeat syntax classification.
Conditional callee tests and discarded sequence prefixes are reads, not invoked
values; selected branches and final sequence values still undergo operation checks.
Logical callee operands are conservatively checked because either can supply a value.

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
