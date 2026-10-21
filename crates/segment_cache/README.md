# segment_cache

Per-file Ecco precursors include owned user-function contracts inferred during
shared-AST extraction. Warm sidecar loads reuse those contracts; linked symbol
resolution and fixed-point effect solving remain build-local.

Source-local parse persistence for `ast_segmenter::parse_module`, separate from
the legacy tracker adapter. It stores the existing owned occurrence facts,
module body and ordered statement ranges, not lossy legacy dependency/name sets.
Syntax-local effect reasons/timing are persisted and range-checked (schema 13 /
extractor 16, including parented execution contexts, complete occurrence timing,
eager read/write links, relative dependency patterns, owned lazy Ecco precursors,
deferred declaration/source-slot alternatives,
dynamic namespace observation shapes/receiver/`this` facts,
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

P2.1 provides a deterministic versioned CBOR codec. SHA-256 identifies the exact
source bytes and protects the payload against accidental corruption. Identity
includes schema/extractor versions, the pinned OXC revision and effective parser
language/module/definition mode. Current parser defaults are part of the
extractor version; bump it when defaults or extraction/support policy change.

`CacheRecord::from_parsed`, `encode` and `decode` validate all UTF-8 ranges,
source-local IDs, links and ordering. Decode limits input to 16 MiB, 2,097,152
CBOR items and 32 levels before serde allocates; indefinite lengths and trailing
data are rejected. This is an untrusted cache, not an authenticated artifact.
Diagnostics are not persisted. The codec is owned after parser arena drop.
`from_parsed` also checks the extraction's sealed source/configuration and record
hashes; passing an unrelated parse alongside plausible-length text is an error.
`to_parsed(source)` returns provenance-bearing owned records for graph inputs,
without parsing or reconstructing legacy tracker maps. Serialized provenance
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
never cached.
These are structural integrity checks on source-local evidence, not a second
parser or authentication of a publisher; source/configuration hashes and bounded
CBOR preflight remain mandatory on both codec boundaries.
Execution contexts validate preorder/nearest parents, original segment containment,
immediate call links and exact reference/invocation coverage. No derived linked
execution constraints or relocation decisions are persisted.

## Sidecar IO

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
provenance for parse-free emission; mutating the public record configuration
cannot override its extraction identity.
Original-only CSS edits may hit the same generated-text record; passing stale
original bytes still fails on that hit. Complete CSS/composition closure checking
belongs to the preparing caller, not resolved state persisted in this cache.

`load_or_parse(logger, path, package_format, CacheMode)` always reads the source,
then checks `<source>.seg.cbor`. It returns the exact source snapshot, record,
hit/miss reason and write outcome. Invalid/unreadable caches are misses. Source
read/parse errors are fatal; old success records are never returned for failures.
No negative diagnostics are cached: failures are regenerated with the current
path. Cache read/write errors are nonfatal and explicitly reported in the
outcome, not logged behind the caller's back. `ReadOnly` suppresses writes;
`Disabled` suppresses both cache reads and writes.

`load_or_parse_with_declarations(..., &LocalFacts)` includes effective contracts
in identity for ESM and opaque CJS; `load_or_parse` uses the default JavaScript
and Node presets. `CacheRecord::matches_declarations` also checks in-memory
identity without parsing. Ecco templates persist only lazy extraction, never
solved hazards; decoding validates their local IDs, UTF-8 spans, source bindings
and unique root-to-segment ownership.

Writes use uniquely named sibling `.seg.cbor.stage-*` files, flush/sync before
atomic replacement, and clean up staging files on errors/normal cancellation.
The pinned tempfile implementation atomically overwrites on Unix and Windows;
it never removes the old destination first. Unsupported-filesystem replacement
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

Watchers should exclude `is_cache_artifact(path)` results. Deleted sources never
load orphan records; the cache does not delete or enumerate orphan sidecars.

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
