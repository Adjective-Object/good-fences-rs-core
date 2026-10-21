# segment_cache

- `records` owns private cache storage; validation/sidecar consumers use accessors.
  Corruption tests edit a serialized test-only projection, never add production
  setters or mutable views of sealed facts.

- Persist `ast_segmenter::owned::ModuleFacts`, body and ordered statement ranges;
  never legacy tracker tags, resolved paths, arena IDs or derived graph/effects.
- Keep split-declarator ownership ranges disjoint; persist and validate shared
  MultiSpan assembly prefixes through emission recipes and provenance.
- Bump extractor identity for parser defaults, extraction or support-policy
  changes; bump schema for incompatible wire layout. Keep OXC revision in sync
  with workspace pins.
- Dynamic namespace observations cover all dynamic dependency IDs in source order;
  property/receiver pairs are sorted and unique. Target safety stays derived.
- Cache ESM top-level suspension spans only; async dependency/SCC state and
  require compatibility belong to the linker/native host, not sidecars.
- Preflight CBOR byte/item/depth limits before serde; validate source hash,
  parser identity, UTF-8 ranges, local IDs, links and order on both boundaries.
- Preserve sealed extraction provenance on both boundaries and in `to_parsed`;
  never bless unrelated parsed facts by hashing separately supplied source text.
- Schema 12 combines execution contexts with typed XXH3 declaration identity.
  Shared `PreparedDeclarations`
  avoids per-file serialization; source/checksum/provenance remain SHA-256.
- Reference `async_deferred` is required syntax evidence covered by provenance,
  not an optimization decision; stale/missing/mutated tags must fail validation.
- Prepared sources cache generated-JS coordinates and parser identity, but recheck
  original resource bytes before sidecar publication and return, including hits.
- Prepared parser path/format overrides resource extensions/package defaults;
  mode-only changes miss even with equal text. Preserve the checked mode in
  sealed provenance through `to_parsed`, never trust mutable record config alone.
- Emission validation checks segment/occurrence coverage, placement and namespace
  links, canonical operation parents, complete nonrecursive source-slot roots,
  PURE input/guard ranges and opaque CJS request/hint-only recipes. Do not reparse.
- Validate parented execution contexts, immediate call links and exact occurrence/
  eager-access coverage; never persist linked scheduling constraints.
- Sidecars use requested lexical source paths; unique sibling staging plus
  closed tempfile paths and std atomic rename, never unlink-before-rename. Last publisher wins;
  recheck source before publish/return. Cache failures are reported nonfatally,
  source/parse failures are fatal. Never clean another writer's staging files.
