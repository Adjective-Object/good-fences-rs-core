# segment_cache

- Persist `ast_segmenter::owned::ModuleFacts`, body and ordered statement ranges;
  never legacy tracker tags, resolved paths, arena IDs or derived graph/effects.
- Bump extractor identity for parser defaults, extraction or support-policy
  changes; bump schema for incompatible wire layout. Keep OXC revision in sync
  with workspace pins.
- Preflight CBOR byte/item/depth limits before serde; validate source hash,
  parser identity, UTF-8 ranges, local IDs, links and order on both boundaries.
