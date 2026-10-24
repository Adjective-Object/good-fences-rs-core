# segment_cache

Per-file Ecco precursors include owned user-function contracts inferred during
shared-AST extraction. Warm sidecar loads reuse those contracts; linked symbol
resolution and fixed-point effect solving remain build-local.

Standard `tracing` spans cover sidecar reads, decode/encode, validation, owned
fact extraction, adapters, source rechecks and publication. They inherit the
caller's subscriber and parent, including bundler preparation workers; this
library installs no subscriber. Only byte counts are recorded, never source
text/paths, declarations or serialized payloads. Recording is opt-in and does
not change cache contents or validity checks. See the bundler README for the
local Chrome/Perfetto trace driver.

Source-local parse persistence for `ast_segmenter::parse_module`, separate from
the legacy tracker adapter. It stores the existing owned occurrence facts,
module body and ordered statement ranges, not lossy legacy dependency/name sets.
`CacheRecord` and `CachedModule` have private storage. Consumers and validators
use borrowed accessors such as `record.module().facts()` and
`record.module().segments()`; no mutable view of sealed records is exposed.
Production loading/publication uses typed FlatBuffers, schema 18/extractor 25.
The last parser-identified source-map directive span is sealed and validated;
resolved upstream maps remain separate output-only bundler inputs.
Facts expose Copy/value-iterator APIs over immutable verified byte owners.
`validate_parsed(source, parsed)` shares the source-local semantic checks with
the production codec and accepts `ParsedModule::from_wire` without decoding a fact
tree. Raw wire callers must apply this check after structural/source verification;
`WireRecord` alone does not prove span/link/recipe validity. `CacheRecord` construction
always runs the same semantic gate before exposing immutable records.
Syntax-local effect reasons/timing are persisted and range-checked (including
parented execution contexts, complete occurrence timing,
eager read/write links, relative dependency patterns, owned lazy Ecco precursors,
deferred declaration/source-slot alternatives,
dynamic namespace observation shapes/receiver/`this` facts,
lazy module/selector construction candidates with composed original argument slots,
per-reference `async_deferred` syntax context,
inferred user-function contracts and declaration identity,
source-local emission recipes/placement constraints,
CommonJS path-parameter declaration policy, ESM top-level await/for-await spans,
effective commonjs format, invocation inputs/annotation sites and normalized member
shapes). No resolver, usage, effects-policy, graph, absolute path or arena IDs
are stored.
Split variable declarators retain disjoint ownership ranges and bounded
`MultiSpan` source-assembly recipes. Shared declaration prefixes survive cache
and graph adapters; provenance covers every piece, and decoding checks UTF-8,
piece order and operation containment without reparsing.
Support-policy/facts changes invalidate previous sidecars. Transitive
async status, SCC schedules and require compatibility remain derived linker/host
policy, never source truth. CJS records cannot carry top-level suspension sites.

The codec stores a size-prefixed `GFSC` root and typed nested `GFSM` module.
SHA-256 identifies the exact
source bytes and protects the payload against accidental corruption. Identity
includes schema/extractor versions, the pinned OXC revision and effective parser
language/module/definition mode. Current parser defaults are part of the
extractor version; bump it when defaults or extraction/support policy change.
Syntax detection for ambiguous CommonJS-default extensions is cached with the
effective module format. Memory/disk hits compare the requested detection policy
without parsing again; package-policy changes or changed source bytes invalidate
the decision. Borrowed semantic validation uses the sealed effective mode.

`CacheRecord::from_parsed` and `decode` validate all UTF-8 ranges,
source-local IDs, links and ordering. Decode limits input to 16 MiB, 2,097,152
tables, 64 levels and 64 MiB apparent size before generated access. Both roots
are structurally verified; unknown discriminants, wrong size prefixes and checksums
are rejected. No Serde, object `unpack`, owned-tree reconstruction or
decode/re-encode canonical comparison occurs. This is an untrusted cache, not an
authenticated artifact. Diagnostics are not persisted.
`from_parsed` also checks the extraction's sealed source/configuration and record
hashes; passing an unrelated parse alongside plausible-length text is an error.
`encode(source)` and `to_parsed(source)` check exact source identity then share/copy
already validated immutable storage, without repeating semantic tree traversal.
`to_parsed` retains byte-backed AST/Ecco facts and lazy statement ranges for graph
inputs, without parsing or reconstructing legacy tracker maps. Serialized provenance
detects accidental re-pairing or mutation, not a malicious cache publisher.
Async-body flags are required on every serialized reference and covered by the
same sealed records hash; missing flags, changed context facts and previous
schema/extractor identities are rejected without reparsing.

Recipe validation covers exact segment/capture/occurrence coverage, declaration,
forward-use and write links, exact eager read/write classification, hoisted availability consistency,
namespace-reference classification links and default/export wrapper containment.
Namespace classification covers all import-binding references, not only `import
* as` syntax, because named/default imports can resolve to namespace reexports.
Operation nesting must be source-ordered canonical preorder with the nearest
containing parent. Each source slot must list exactly its composed roots, never
itself or an ancestor. PURE inputs link back to the invocation's original
callee/arguments; chain slots remain ordered within that callee and guard
boundaries/known module-await vetoes are checked. CJS recipes permit only
dependency and source-hint slots, never lexical or invocation rewrites.
Deferred alternatives validate declaration binding links and complete initializer
source-slot roots; CommonJS cannot carry these ESM reconstruction alternatives.
Dynamic namespace observations cover exactly the dynamic dependency occurrences,
with sorted unique property/receiver pairs; placement and target eligibility are
never cached. Schema 14 added typed lazy construction candidates; extractor 19
excludes defaulted getters and validates complete synchronous arrow/return token
boundaries against canonical invocation, operation and execution spans, without
parsing or unpacking on hits.
Their borrowed validation checks segment/declaration/constructor/dependency and
module/getter binding links, exact original argument spans, complete composed
source-slot roots, ordering and wrapper spelling hints. Aliased or shadowed
constructors remain syntax candidates, never runtime identity proof. Returned-import
and complete property-token spans anchor dependency selection and Unicode-decoded
property names to the original source without reparsing; receiver/dot trivia and
token boundaries reject substring or comment-token substitutions. Unsupported
getters retain ordinary references; options remain one original source slot.
Schema 15 adds optional complete plain-options property evidence. Extractor 21
omits that evidence for JSX and TypeScript type/generic values unsupported by
the borrowed boundary proof, preserving ordinary candidates and all source facts.
Schema 16 adds exact initialized-result binding/declarator links. Extractor 23
omits optional result-boundary evidence for annotated TS declarators rather than
misinterpreting type tokens. Nested result liveness remains a derived linker
decision, never a cached retention conclusion.
Borrowed key/value and punctuation checks exclude initializer/prototype keys,
computed/spread/accessor/method forms and incomplete property lists without
AST parsing. Unknown options, escaped keys, parenthesized objects and regexp/division
values conservatively lack evidence. Effectful values and captures are preserved,
not declared pure; registered runtime identity remains a build-local decision.
Fresh and wire facts agree without parsing or unpacking on hits. Lazy runtime
recognition, all-reference certification, graph transformations and emission plans
are deliberately absent.
These are structural integrity checks on source-local evidence, not a second
parser or authentication of a publisher; source/configuration hashes and bounded
wire verification remain mandatory at the codec boundary.
Execution contexts validate preorder/nearest parents, original segment containment,
immediate call links and exact reference/invocation coverage. No derived linked
execution constraints or relocation decisions are persisted.

## Sidecar IO

Source/extraction failures retain operation/path context and underlying error
chains. Nonfatal cache read/write diagnostics include the sidecar operation/path;
cache failures still never authorize stale source facts.

`load_prepared(logger, path, PreparedSource, mode, declarations)` supports resource
loaders. `PreparedSource` separates exact original input bytes from generated
JavaScript text and its parser path/format. Sidecars remain next to the original
input, but facts, spans, hashes and sealed provenance describe the generated
JavaScript. The caller must prepare deterministically from the current input;
both publication and return recheck the original bytes, including on cache hits.
No generated source is written to the input path.
The prepared parser path/format, not the original resource extension or package
mode, keys sidecar reuse. Changing only that effective mode is a cache miss even
for identical generated text. `to_parsed` preserves that checked mode in sealed
provenance for parse-free emission; tampering with serialized configuration
cannot override its extraction identity.
Original-only CSS edits may hit the same generated-text record; passing stale
original bytes still fails on that hit. Complete CSS/composition closure checking
belongs to the preparing caller, not resolved state persisted in this cache.

`load_or_parse(logger, path, package_format, CacheMode)` always reads the source,
then checks `<source>.seg.fb`. It returns the exact source snapshot, record,
hit/miss reason and write outcome. Invalid/unreadable caches are misses. Source
read/parse errors are fatal; old success records are never returned for failures.
No negative diagnostics are cached: failures are regenerated with the current
path. Cache read/write errors are nonfatal and explicitly reported in the
outcome, not logged behind the caller's back. `ReadOnly` suppresses writes;
`Disabled` suppresses both cache reads and writes.
If a valid owned extraction exceeds wire byte/depth/table limits, it remains an
uncached success and publication reports `CacheWrite::Failed`, preserving any old
sidecar. This exceptional cold record retains checked owned facts; disk hits
always retain verified wire storage. Untrusted wire input cannot use this fallback.

`load_or_parse_with_declarations(..., &LocalFacts)` includes effective contracts
in identity for ESM and opaque CJS; `load_or_parse` uses the default JavaScript
and Node presets. `CacheRecord::matches_declarations` also checks in-memory
identity without parsing. Ecco templates persist only lazy extraction, never
solved hazards; decoding validates their local IDs, UTF-8 spans, source bindings
and unique root-to-segment ownership.

Shared configurations use `ast_segmenter::PreparedDeclarations` with
`load_or_parse_with_prepared_declarations`, `load_prepared_with_declarations`,
`CacheRecord::matches_prepared_declarations` and
`CacheRecord::decode_with_prepared_declarations`. These paths compare the prepared
identity without serializing declarations per source or hit. Existing `LocalFacts`
entrypoints remain compatibility wrappers. Declaration identity remains the
typed 128-bit XXH3 fingerprint; its independent CBOR configuration serialization
is retained for compatibility, once per prepared configuration, not per-file
loading. Debug/test Serde projections are not the production codec.
Source, payload checksum and sealed extraction integrity remain SHA-256.

Writes use uniquely named sibling `.seg.fb.stage-*` files, flush/sync before
atomic replacement, and clean up staging files on errors/normal cancellation.
Staging handles close before `std::fs::rename` atomically overwrites the
destination, including Windows replacement with open readers. It never removes
the old destination first. Unsupported-filesystem replacement
errors preserve the previous cache and return an uncached parse. A crash may
leave a staging orphan: readers ignore it, and no writer deletes another
writer's staging file. This promises atomic visibility, not directory-fsync
durability after power loss.

Concurrent writers are independent, last successful atomic replacement wins.
Hash/config validation makes stale publications safe misses, never stale hits.
Source bytes are checked again before publication and before return; detected
changes/deletion return an error and cannot publish the parsed snapshot. As
with any filesystem snapshot API, a noncooperating editor may change the source
after the last check; there is no cross-file transaction with external writers.
No source is modified: read-only source files work when the parent is writable,
and unwritable parents fall back to parsing with a reported cache-write error.
Sidecars follow the requested lexical source path (including symlink aliases),
not a canonicalized target. Caller-supplied effective package policy remains
authoritative; callers own resolution/symlink/casing policy.

Watchers exclude `is_cache_artifact(path)` results: both `.seg.fb` and legacy
`.seg.cbor` sidecars and their staging siblings. Legacy sidecars are explicit
miss/rebuild inputs, never decoded as current records or deleted. Legacy data
placed at a new suffix is an invalid-record miss. Deleted sources never load
orphan records; the cache does not delete or enumerate orphan sidecars.

## Invalidation boundaries

Source hashes, not mtimes, decide hits. Relocating identical source/sidecar bytes
is a hit when effective parser policy matches; consumers must resolve each raw
dependency again using the new root. Package `type` is discovered by the caller
and passed as `ModuleFormat`; `.mjs`/`.mts` and `.cjs`/`.cts` still override it.
JS/TS/JSX/definition or effective module-mode changes miss even with identical
source bytes. Resolver extensions/conditions, package exports, `sideEffects`
metadata are deliberately absent from this cache. Those changes invalidate
derived resolution/effects later, not source facts. Effect DSL configuration
keys extraction and invalidates source precursors when declarations change.
PURE hint spans and syntactic value-use contexts are source facts; whether a
hint permits an invocation rewrite is derived and is never persisted here.
Emission recipes likewise contain only original-coordinate operations and
occurrence links, precomputed nesting, connector/placement/name constraints and
optional PURE lowering shapes. They contain neither chosen identifiers nor
resolved requests, output variants, graph ownership or unconditional purity.

Cold/warm and disabled-cache results have identical owned facts/body/ranges;
the warm API intentionally does not reconstruct legacy `Segment` name/tag maps.
Legacy tracker parsing is untouched and does not automatically opt into this
bundler-policy cache. There is no P3 graph integration or P4 effects evaluator
in this crate.

Run `cargo test -p segment_cache` for codec checks.
