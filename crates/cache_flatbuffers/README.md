# Typed source-cache FlatBuffers

**Production source-cache backend: schema 22/extractor 40, `.seg.fb`.**
`segment_cache` reads these immutable buffers through AST/Ecco views and runs
full semantic validation before exposing checked records. Byte-backed parsed
modules feed graph linking, effect solving and native emission without unpacking
facts. Encoders and generated object APIs are construction-only.
Independent resolution aggregates can be rebuilt directly from verified borrowed
request tables; preserving existing requests never reconstructs an owned proof
tree. The bundler revalidates filesystem semantics before reuse/publication.

## Schema coverage

The schemas in the repository's `flatbuffers` directory represent the complete
persisted record shape, rather than embedding CBOR bytes:

| Schema | Persisted records represented |
| --- | --- |
| `common.fbs` | `ParserConfig`; fixed-width spans, XXH3-128, declaration fingerprints; optional IDs without sentinel collisions |
| `ecco.fbs` | `HazardFields`, `Update`, `Definition`, `LocalFacts`, `GlobalSymbol`, `FunctionSignature`, `CallResult`, `OwnedFunction`, `ParsedSymbol`, `ExternalReference`, `ParsedRoot`, `ParsedValue`, `DefinitionMatch`, `ParsedFile` |
| `syntax.fbs` | `Binding`, `Reference`, `Dependency`, `ImportBinding`, `Export`, `Directive`, `EffectSite`, `IdentifierCallee`, `InvocationArgument`, `Invocation`, `ExecutionContext`, `ExecutionFacts`, `PlacementFacts`, `NameRestrictions`, `NamespaceUse`, `SourceSlot`, `CalleePlan`, `PureLowering`, `Operation`, `SegmentRecipe`, `DynamicNamespace`, `LazyModuleCandidate`, `LazyOptionProperty`, `LazySelectorCandidate`, `EmissionFacts`, `PolicyViolation`, `ModuleFacts` |
| `module.fbs` | `CachedModule`, ESM/CommonJS body alternatives, ordered statement spans |
| `cache.fbs` | `SourceProvenance`, `CacheRecord` |
| `style.fbs` | Independent stylesheet original/prepared/config identities, URL occurrences and CSS Modules names/scopes/compositions/animations |
| `type_only_facts.fbs` | Independent original-TypeScript declarations, exported type/value/const-enum presence and routes, original import/reexport uses, ordered enum constants and typed diagnostics |

This covers persisted source records, plus explicit union alternatives for
imports, exports, callees, namespace accesses, operations, export forms, chain
steps, Ecco bindings, recursive hazard expressions, arguments, returns, and all
13 Ecco instruction variants. `MultiSpan` uses a checked one-to-four span vector.
Interpreter-local `usize` identities use explicit `ulong` fields; source-local
byte offsets remain `uint`. Ecco effect sources use strictly increasing keys.
Declaration fingerprints retain their 16-byte representation, not a new hash.

## Buffer format and read guarantees

The `.seg.fb` format is a size-prefixed `CacheRecord` FlatBuffer (`GFSC`)
followed by a 16-byte big-endian XXH3-128 of the entire size-prefixed record. Its `module`
vector contains a **typed `CachedModule` FlatBuffer** (`GFSM`). Nesting enables
an exact immutable module-byte extraction seal without serializing the facts
again during provenance verification. It is not a CBOR envelope.

`WireRecord::new` verifies the checksum,
size prefix, identifier, schema/extractor/parser identities, source hash/length,
configuration, and extraction module digest. Both roots independently receive
FlatBuffers runtime verification with bounded depth and tables, and apparent
size bounded to four times the actual buffer length. Source facts and standalone
Ecco buffers have no fixed byte cap; wire-format offset/span limits still apply.
Independent resolution and upstream-map records retain their 16 MiB limits.
A borrowed traversal then rejects unknown enum/union discriminants, absent union
values, invalid MultiSpan lengths, duplicate hazard fields, and unordered/duplicate
effect-map keys. No object API `unpack` is called.

`WireStyle` uses independent schema 1/extractor 1 (`GFST`), with original and
prepared source hashes, extraction configuration and nested typed facts integrity.
Structural verification and UTF-8 span/containment/discriminant checks precede
borrowed access. Stylesheet facts have no fixed byte cap and production loads use
the same private mapped backing; resolved targets and linked names are not cached.

`WireTypeOnlyFacts` uses independent schema/extractor 1 (`GFTY`), sealing original
TypeScript bytes, selected syntax/package/extractor settings and nested typed facts
with big-endian XXH3-128. Both roots receive bounded structural verification;
borrowed semantic validation checks discriminants, UTF-8/owner spans, ordering,
local/export presence consistency, enum links and constant payload/diagnostic
requirements before construction. Optional numeric scalars preserve negative zero.
Expanded enum strings are capped at 64 KiB each and 4 MiB total, with checked
arithmetic both before packing owned facts and during borrowed cache validation.
These limits do not cap original source bytes or the entire fact tree.
Its `facts()` tables borrow immutable private backing; object APIs are cold
construction only. `encode_in` requires an explicit writable snapshot directory.
The bundler separately owns scoped source observations and freshness validation.
These records contain no graph resolution, solved exports or repair decisions and
do not change source schema 22/extractor 40.

The owner retains `SharedBytes`, a payload endpoint and the verified module
range. `WireRecord`, `WireEcco` and `WireSourceMap` accept the same backing.
Production script records use an Arc-shared read-only file mapping; low-level
constructors still accept heap bytes. `SharedBytes::snapshot` streams into a
private file in the OS temporary directory (or an explicit directory with
`snapshot_in`), never maps mutable source/cache paths, and never falls back to
heap storage. Unix files are successfully unlinked before mapping; Windows
files deny sharing and delete on close. Mappings drop before their private file
handles, including after the last clone, and empty snapshots need no OS mapping.
Warm loads pay a disk-to-private-file copy for safety against external mutation,
truncation and atomic replacement, but never allocate a whole-file heap buffer.
Views borrow the owner; cloning it only shares bytes. Small documented
unchecked-root calls reuse the already verified immutable roots. There is no
lifetime transmute or self-referential storage. Tests prove that nested string
pointers address this backing and remain identical across thread-shared clones.
`WireSourceMap::snapshot` similarly moves a verified map, including optional
embedded source contents, to private mapped storage without unpacking or resealing.

**`WireRecord` is not the source-local semantic validator.** Span UTF-8
boundaries, ID/link/occurrence coverage, ordering, operation-parent rules,
execution contexts, supported recipes and Ecco semantic bounds are checked by
`segment_cache::validate_parsed` through owned-or-wire views. Call that gate after
`ParsedModule::from_wire`; structural/source verification alone is insufficient.

AST's `ParsedModule::to_flatbuffer` verifies owned extraction provenance before
constructing typed tables, embedding Ecco directly with its existing encoder.
`FactsStorage` retains the verified owner; strings and nested records borrow it,
while graph ranges and derived analysis/emission plans are explicitly owned.
AST and bundler differential tests cover complete record equality, byte borrowing,
owner lifetime, execution/recipe validation, ESM/CJS outputs and parallel emission.

Schema 22/extractor 40 are active production identities, included in bundler
source-parse fingerprints. Atomic sibling publication uses `.seg.fb.stage-*`;
inventory ignores current and legacy `.seg.cbor` artifacts, without deleting them.
Old formats miss/rebuild. This crate's handwritten wire path does not use Serde.
Owned AST provenance now hashes typed module bytes too; CBOR remains only in the
independent declaration-configuration fingerprint and debug/test fixtures.

Lazy module/selector tables persist only source-local binding/reference links,
spelling-only wrapper hints and original composed argument slots. Options and
getters remain source spans, never duplicated expressions or transformation
plans. Known wrapper discriminants are checked before borrowed access; semantic
span/link/slot validation remains in `segment_cache`. Existing sidecars rebuild
under the new schema/extractor identities.
Dependencies retain a typed static JSON import-attribute clause span. Borrowed semantic
validation restricts it to static ESM imports; the build validates target resource
kind after resolution, including cache hits.
Typed `PolicyViolation` records preserve stable `PolicyName` discriminants and
UTF-8 source spans, never caller admission or rendered diagnostics. Unknown
discriminants fail structural validation; span/order checks remain semantic.
Consumers enforce their current policy on borrowed evidence without reparsing.

Parser configuration stores the effective module format and whether ambiguous
CommonJS-default inputs use syntax detection. Requested policies can accept a
cached detected ESM mode without parsing again; language/definition/JSX flags and
detection policy must still match. The cached root and extraction provenance
remain exactly equal, and module body, facts and Ecco mode are validated against
that sealed effective format.
Optional `LazyOptionProperty` key/value spans describe a complete plain literal
object without initializer/prototype properties, never effect-free values.
Unknown options have no structural proof; linking/certification remains derived.
Selectors also retain optional AST-proven initialized-result binding and complete
declarator evidence, checked before borrowed consumers inspect captured reads.

## Production upstream-map metadata

`WireSourceMap` stores typed tokens, original/resolved source URLs, nullable
contents, names, file and sourceRoot in verified `GFUM` FlatBuffers with XXH3-128
integrity. Script bytes, input bytes and map location are sealed independently
of AST extraction. Bounded structural and borrowed semantic checks reject invalid
ordering and indices before construction. Warm `.upstream.seg.fb` readers retain
immutable bytes and use same-line greatest-lower-bound lookup without JSON/DTO
unpacking, preserving explicit unmapped boundaries.

Ecco's precursor substage is usable independently: `WireEcco`
structurally verifies a standalone typed `ParsedFile`, then owns its immutable
bytes. `validate_ecco` also checks discriminants, effect-source order and ID
widths for embedded roots. Ecco's own view-based semantic validation remains a
separate gate before analysis. Function templates retain byte owners and read
instructions directly, while mutable interpreter state may allocate.
The `CallInstruction.pure` field represents the invocation annotation override,
not construction; its schema name now matches the existing interpreter meaning.

## Independent resolution records

`resolution.fbs` defines schema **2** (`GFRS`), independent of source schema
22/extractor 40. `resolution::WireResolution` retains private `Arc<[u8]>` bytes.
Source/map hashes, extraction module seals, filesystem proof content seals and
all three framing checksums use 16-byte big-endian XXH3-128, matching existing
fingerprint byte order. These are corruption checks, not authentication.
Resolution package/dependency/policy keys remain 32-byte SHA-256 digests.
Earlier formats are rejected and rebuilt; no padded digests or legacy decoding.
Size/checksum/framing, bounded verification, all discriminants, portable anchors,
proof shape and strict request ordering/uniqueness are checked before exposure.
Import/require requests bind package-instance, named-dependency and policy hashes
to typed file/external targets and ordered positive/negative filesystem proofs.
Paths use runtime package/project/working-directory anchors; explicit ancestor
anchors cover Node lookup
directories and ancestor manifests, never arbitrary external machine paths.
Filesystem validation remains the consuming backend's separate checked gate.
Generated object APIs are cold construction only; warm views never unpack.

## Regeneration

Compiler **flatc 25.2.10**, runtime **flatbuffers = "=25.2.10"**.
Generated Rust is checked into `src/cache_generated.rs`; ordinary Cargo builds
neither invoke flatc nor download it. The optional object API is for construction.

From the repository root, using the official `google/flatbuffers` release
`v25.2.10` Windows compiler:

```powershell
$env:FLATC = 'C:\path\to\flatbuffers-tools\flatc.exe'
.\scripts\generate-cache-flatbuffers.ps1
```

The script checks the exact compiler version and generates all bindings.
Use `-Schema Source`, `Resolution`, `Style`, or `TypeOnly` to generate only one.
The source invocation remains:

```text
flatc --rust --gen-all --gen-object-api -o crates\cache_flatbuffers\src flatbuffers\cache.fbs
```

The downloaded official `Windows.flatc.binary.zip` used for this output has SHA-256
`bd9b3cb6bf5ab8009f1927b6e00207db968c3d5c93e3bc215dae084cc681249a`.
No compiler installation or global configuration change is required.

Focused validation:

```text
cargo test -p cache_flatbuffers
cargo clippy -p cache_flatbuffers --all-targets -- -D warnings
```

Tests cover borrowed reads, immutable sharing, optional zero IDs, every union
family, recursive contracts, ESM/CJS separation, stale identities, source/config
and extraction-seal mismatches, corrupted/truncated roots and vectors, legacy
non-FlatBuffer data, depth limits, large facts, unknown discriminants and ordered maps.
Synthetic wire-shape tests are not a substitute for executable cache integration
tests.
