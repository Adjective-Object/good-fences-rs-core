# cache_flatbuffers

- Independent `resolution.fbs` schema 1 owns verified borrowed `GFRS` records.
  Package/dependency/policy identities and typed scoped proofs do not change
  source schema/extractor versions. Consumers freshly validate filesystem proofs
  before checked reuse; generated object APIs remain construction-only.
  Aggregate updates copy verified borrowed request tables directly into a new
  builder, preserving untouched requests without unpacking input/proof trees.

- This is the production `.seg.fb` backend (schema 19/extractor 30).
  `segment_cache` adds source-local semantic validation before exposing records.
- Parser config seals effective format and syntax-detection policy. Requested
  CommonJS-default detection may match cached ESM only under identical policy;
  root/provenance configs and module/facts/Ecco format must still agree exactly.
- Upstream maps use verified typed `WireSourceMap` bytes in `.upstream.seg.fb`,
  with script/input/location seals, bounded verification and borrowed token/index
  validation. Preserve null contents and unmapped tokens; never unpack on hits.
- `flatbuffers/*.fbs` is authoritative. Keep `cache_generated.rs` byte-for-byte
  compiler output; regenerate using pinned flatc 25.2.10, matching the runtime.
  Only this generated module has narrowly scoped generated-code lint expectations.
- `WireRecord` owns immutable shared bytes, not decoded DTOs. Verify both outer
  and nested roots with bounded runtime verification before any generated reads.
  Reject unknown discriminants independently; FlatBuffers permits them by default.
- `WireEcco` provides the same immutable verified ownership for standalone
  precursors; Ecco validates semantic links through borrowed views before solve.
- The nested module is itself a typed FlatBuffer, never CBOR or an opaque DTO.
  Its exact SHA-256 is the extraction seal; the outer checksum covers framing and
  metadata. Generated object APIs are construction-only, never warm-load adapters.
- Wire/source integrity is not semantic validity. Production integration must
  retain source-local span/link/ordering/recipe checks and private extraction seals.
  Do not manufacture seals around arbitrary source/facts or deserialize for checking.
- AST construction/views live in `ast_segmenter`; shared `segment_cache` validators
  are the owned/wire semantic gate. Graph/Ecco/emission retain byte owners.
- Lazy candidates are typed source-local tables with original composed slots;
  wrapper-kind hints require discriminant validation and prove no runtime identity.
- Optional lazy-options property tables describe complete plain-own-property
  literal shape only; values retain their original evaluation and captures.
