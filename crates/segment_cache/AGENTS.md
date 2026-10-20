# segment_cache

- Persist `ast_segmenter::owned::ModuleFacts`, body and ordered statement ranges;
  never legacy tracker tags, resolved paths, arena IDs or derived graph/effects.
- Bump extractor identity for parser defaults, extraction or support-policy
  changes; bump schema for incompatible wire layout. Keep OXC revision in sync
  with workspace pins.
- Cache ESM top-level suspension spans only; async dependency/SCC state and
  require compatibility belong to the linker/native host, not sidecars.
- Preflight CBOR byte/item/depth limits before serde; validate source hash,
  parser identity, UTF-8 ranges, local IDs, links and order on both boundaries.
- Preserve sealed extraction provenance on both boundaries and in `to_parsed`;
  never bless unrelated parsed facts by hashing separately supplied source text.
- Emission validation checks segment/occurrence coverage, placement and namespace
  links, canonical operation parents, complete nonrecursive source-slot roots,
  PURE input/guard ranges and opaque CJS request/hint-only recipes. Do not reparse.
- Sidecars use requested lexical source paths; unique sibling staging plus
  tempfile atomic replacement, never unlink-before-rename. Last publisher wins;
  recheck source before publish/return. Cache failures are reported nonfatally,
  source/parse failures are fatal. Never clean another writer's staging files.
