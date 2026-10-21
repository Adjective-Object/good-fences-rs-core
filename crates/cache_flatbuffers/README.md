# Typed source-cache FlatBuffers

**Production source-cache backend: schema 15/extractor 21, `.seg.fb`.**
`segment_cache` reads these immutable buffers through AST/Ecco views and runs
full semantic validation before exposing checked records. Byte-backed parsed
modules feed graph linking, effect solving and native emission without unpacking
facts. Encoders and generated object APIs are construction-only.

## Schema coverage

The schemas in the repository's `flatbuffers` directory represent the complete
persisted record shape, rather than embedding CBOR bytes:

| Schema | Persisted records represented |
| --- | --- |
| `common.fbs` | `ParserConfig`; fixed-width spans, SHA-256, declaration fingerprints; optional IDs without sentinel collisions |
| `ecco.fbs` | `HazardFields`, `Update`, `Definition`, `LocalFacts`, `GlobalSymbol`, `FunctionSignature`, `CallResult`, `OwnedFunction`, `ParsedSymbol`, `ExternalReference`, `ParsedRoot`, `ParsedValue`, `DefinitionMatch`, `ParsedFile` |
| `syntax.fbs` | `Binding`, `Reference`, `Dependency`, `ImportBinding`, `Export`, `Directive`, `EffectSite`, `IdentifierCallee`, `InvocationArgument`, `Invocation`, `ExecutionContext`, `ExecutionFacts`, `PlacementFacts`, `NameRestrictions`, `NamespaceUse`, `SourceSlot`, `CalleePlan`, `PureLowering`, `Operation`, `SegmentRecipe`, `DynamicNamespace`, `LazyModuleCandidate`, `LazyOptionProperty`, `LazySelectorCandidate`, `EmissionFacts`, `ModuleFacts` |
| `module.fbs` | `CachedModule`, ESM/CommonJS body alternatives, ordered statement spans |
| `cache.fbs` | `SourceProvenance`, `CacheRecord` |

This covers persisted source records, plus explicit union alternatives for
imports, exports, callees, namespace accesses, operations, export forms, chain
steps, Ecco bindings, recursive hazard expressions, arguments, returns, and all
13 Ecco instruction variants. `MultiSpan` uses a checked one-to-four span vector.
Interpreter-local `usize` identities use explicit `ulong` fields; source-local
byte offsets remain `uint`. Ecco effect sources use strictly increasing keys.
Declaration fingerprints retain their 16-byte representation, not a new hash.

## Buffer format and read guarantees

The `.seg.fb` format is a size-prefixed `CacheRecord` FlatBuffer (`GFSC`)
followed by a 32-byte SHA-256 of the entire size-prefixed record. Its `module`
vector contains a **typed `CachedModule` FlatBuffer** (`GFSM`). Nesting enables
an exact immutable module-byte extraction seal without serializing the facts
again during provenance verification. It is not a CBOR envelope.

`WireRecord::new` bounds the complete record to 16 MiB, verifies the checksum,
size prefix, identifier, schema/extractor/parser identities, source hash/length,
configuration, and extraction module digest. Both roots independently receive
FlatBuffers runtime verification with bounded depth, tables, and apparent size.
A borrowed traversal then rejects unknown enum/union discriminants, absent union
values, invalid MultiSpan lengths, duplicate hazard fields, and unordered/duplicate
effect-map keys. No object API `unpack` is called.

The owner retains only an `Arc<[u8]>`, a payload endpoint and the verified module
range. Views borrow the owner; cloning it only shares bytes. Two small documented
unchecked-root calls reuse the already verified immutable roots. There is no
lifetime transmute or self-referential storage. Tests prove that nested string
pointers address this allocation and remain identical across thread-shared clones.

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

Schema 15/extractor 21 are active production identities, included in bundler
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
Optional `LazyOptionProperty` key/value spans describe a complete plain literal
object without initializer/prototype properties, never effect-free values.
Unknown options have no structural proof; linking/certification remains derived.

Ecco's precursor substage is usable independently: `WireEcco` bounds and
structurally verifies a standalone typed `ParsedFile`, then owns its immutable
bytes. `validate_ecco` also checks discriminants, effect-source order and ID
widths for embedded roots. Ecco's own view-based semantic validation remains a
separate gate before analysis. Function templates retain byte owners and read
instructions directly, while mutable interpreter state may allocate.
The `CallInstruction.pure` field represents the invocation annotation override,
not construction; its schema name now matches the existing interpreter meaning.

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

The script checks the exact compiler version and executes:

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
non-FlatBuffer data, size/depth limits, unknown discriminants and ordered maps.
Synthetic wire-shape tests are not a substitute for executable cache integration
tests.
