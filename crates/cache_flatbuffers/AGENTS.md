# cache_flatbuffers

- The production cache is not integrated yet. Do not describe schema availability
  or structural verification as completion of the per-file storage migration.
- `flatbuffers/*.fbs` is authoritative. Keep `cache_generated.rs` byte-for-byte
  compiler output; regenerate using pinned flatc 25.2.10, matching the runtime.
  Only this generated module has narrowly scoped generated-code lint expectations.
- `WireRecord` owns immutable shared bytes, not decoded DTOs. Verify both outer
  and nested roots with bounded runtime verification before any generated reads.
  Reject unknown discriminants independently; FlatBuffers permits them by default.
- The nested module is itself a typed FlatBuffer, never CBOR or an opaque DTO.
  Its exact SHA-256 is the extraction seal; the outer checksum covers framing and
  metadata. Generated object APIs are construction-only, never warm-load adapters.
- Wire/source integrity is not semantic validity. Production integration must
  retain source-local span/link/ordering/recipe checks and private extraction seals.
  Do not manufacture seals around arbitrary source/facts or deserialize for checking.
