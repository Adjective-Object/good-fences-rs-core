# ast_segmenter

## Parser stack

Uses the oxc parser (`oxc_parser`) with `oxc_semantic` for scope analysis.

## Extract-then-drop arena pattern

oxc allocates its AST in a bumpalo arena (`oxc_allocator::Allocator`). All
`Atom<'a>`, `&'a Node`, and `oxc_allocator::Vec` types borrow from the arena
and cannot outlive it.

Parse pipeline:

1. Borrow (or create) an `Allocator`.
2. Parse → run `SemanticBuilder` → run visitors that copy owned data out.
3. Drop (or `reset()`) the `Allocator`.

Returned types (`Segment`, `RawModuleDeps`, `VariableScope`) hold only owned
data (`String`, `CompactString`, `oxc_span::Span`). Nothing borrows `'a`.

In threaded contexts, each Rayon worker holds a `thread_local! { Allocator }`
and calls `Allocator::reset()` between files to reuse memory.

## Scope analysis

`oxc_semantic` replaces the old hand-rolled `ast_name_tracker`. `SymbolFlags`
maps to `HoistingLevel` via `hoisting_for_flags()`.
Escaped references retain OXC's type/value distinction; a name is type-only
only if all its references in the segment are type-only. Do not discard runtime
references when merging names.

## Compatibility spans and graph

Statement spans are zero-based, end-exclusive UTF-8 byte offsets. The legacy
`SegmentGraph` treats execution-only imports and opaque requires as dependencies
on all target statements, not only exports; this is not evaluation scheduling.
Split top-level multi-declarator variables (including named exports) into
disjoint declarator ownership ranges. Cached recipe `source: MultiSpan` owns
assembly pieces, including shared prefixes; never use an ownership range alone
to reconstruct split declarations. Keep destructuring and CJS bodies indivisible.

## Bundler parse policy

`parse_module` uses explicit package format and extension-sensitive language
modes; validate parser and semantic diagnostics before extraction. It returns
owned records or an error, never a skipped/partial executable program. Keep the
legacy unused-tracker adapter separate. TS/JSX parsing is not transformation.

`ParsedModule::facts` is the occurrence-preserving bundler IR; legacy
`RawModuleDeps` remains a lossy compatibility adapter. Binding IDs are first
declaration byte offsets and dependency IDs are site byte offsets, local to
the exact source/configuration. Keep all declaration/reference spans and
type/value metadata; never expose OXC IDs. Sort DTO collections by source
position before returning.
Relative template/concatenation loaders retain one real occurrence, literal
pieces separated by runtime gaps and the original argument span. Never persist
resolved candidates or fabricate dependency IDs for closure capture.
Keep the effective commonjs format in facts so graph/cache consumers never
infer package format from extension, wrapper references or export syntax.
Extraction provenance binds source bytes, effective parser config and owned
body/ranges/facts. Preserve it through cache/graph adapters; hashes detect
accidental re-pairing, not publisher forgery.

`ModuleBody::CommonJs` always has one opaque executable segment excluding
hashbang, never independently tree-shakable internal statements. Classify
unresolved wrapper references separately from lexical shadows; dependency
function containment is syntax metadata, not eager evaluation permission.
`policy` is the checked source-local support gate; target format, assets and
externals remain resolver/linker checks. Low-level parsing and legacy adapters
intentionally accept a broader syntax set.
CommonJS path parameters allow var/function redeclarations and nested shadows;
reject top-level lexical redeclarations before factory emission.
`top_level_await` owns ESM await/for-await source spans outside functions;
it is not async dependency/SCC status. CJS TLA stays rejected.

`effects` stores syntax-only reason sites, including deferred function/default-
argument and instance-field timing. Never turn annotations or caller policy into
cached unconditional purity. Unknown syntax/TDZ/property operations stay
conservative; opaque CJS is always one effectful body.

`invocation` owns call/new inputs, chain/reference shape, syntax value-use context
and precisely attached PURE hint spans. Only the derived bundler evaluator may
discard an unused invocation; input effects and optional guards must survive.

`facts.emission` owns segment-indexed recipes and placement/name constraints.
`facts.ecco` owns lazy ESM extraction from the same AST with unique segment
root ownership; CJS stays `None`. Effective `ecco_declarations_hash` is a sealed
configuration input in both formats, never graph conclusions. Share immutable
`PreparedDeclarations` across checked parses/cache checks; its typed XXH3 identity
is computed from validated facts, never paired with a caller-supplied digest.
Source/body/facts provenance keeps SHA-256 independently.
Operations address existing occurrence IDs/indices and original spans; preorder
parents and source-slot roots precompose nesting before the arena drops.
PURE callee steps preserve parentheses as chain boundaries and guard awaits in
the invocation's own function scope. CJS remains one opaque recipe with only
dependency/hint slots. Never persist chosen names, targets or transform policy.
Namespace-use candidates cover all import bindings; only linked namespace
identities activate them, including named/default imports of namespace reexports.
