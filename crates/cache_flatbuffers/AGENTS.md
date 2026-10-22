# cache_flatbuffers

- This is the production `.seg.fb` backend (schema 16/extractor 23).
  `segment_cache` adds source-local semantic validation before exposing records.
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
