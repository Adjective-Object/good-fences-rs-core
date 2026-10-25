# segment_cache

- `records` owns private cache storage; validation/sidecar consumers use accessors.
  Corruption tests edit a serialized test-only projection, never add production
  setters or mutable views of sealed facts.
- Production `.seg.fb` records retain verified immutable FlatBuffers. Construction
  runs full borrowed semantic validation once; adapters recheck exact source
  identity and share owners, never unpack or re-encode for canonical comparison.

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
- Bound FlatBuffer byte/depth/table/apparent-size verification; validate source hash,
  parser identity, UTF-8 ranges, local IDs, links and order on both boundaries.
- Preserve sealed extraction provenance on both boundaries and in `to_parsed`;
  never bless unrelated parsed facts by hashing separately supplied source text.
- Schema 19/extractor 28 retains typed nested module buffers and wire seals,
  including dynamic namespace observations, deferred recipes, async references
  and lazy construction candidates with original argument slots.
- URL occurrences validate construction/global-reference/base/literal evidence;
  their composed dependency operations address only the original literal slot.
- The two-empty-piece dynamic pattern captures the importing directory tree;
  require patterns still need a relative prefix. Ignored imports have no dependency
  occurrence, but retain ordinary source/input effect and reference facts.
- Persist effective syntax-detected format alongside its requested detection
  policy. Match requests without reparsing; run semantic validation against the
  cached sealed effective config, never the unresolved package fallback.
- The last parser-identified source-map directive span is extraction evidence;
  validate its UTF-8 range and directive syntax without reparsing. Actual maps
  are output-only bundler inputs independent of JS extraction.
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
- Lazy candidates validate segment/declaration/constructor/import/getter links,
  original argument spans, complete composed slot roots and spelling-only kind
  hints. Runtime identity and all-reference certification remain linker policy.
  Anchor directly returned imports and decoded property tokens to explicit source
  spans; nested option imports must not replace callback dependency identities.
  Token-boundary checks prove complete synchronous concise-arrow returns and
  simple nondefaulted parameters; containment alone never certifies lazy syntax.
  Optional options-property evidence validates complete object/key/value
  boundaries and excludes initializer/prototype keys without reparsing values.
  Result-binding evidence links the exact initialized identifier and complete
  declarator to its construction, including parenthesized initializer boundaries.
  Annotated TS declarators omit this optional evidence; do not reject otherwise
  valid cached syntax or infer initializer boundaries from type tokens.
- Sidecars use requested lexical source paths; unique sibling staging plus
  closed tempfile paths and std atomic rename, never unlink-before-rename. Last publisher wins;
  recheck source before publish/return. Cache failures are reported nonfatally,
  source/parse failures are fatal. Never clean another writer's staging files.
- Valid owned extraction that exceeds wire limits remains an uncached success;
  never allow untrusted wire input through that cold-only fallback.
- Exclude both `.seg.fb`/`.seg.cbor` and their staging siblings from inventory;
  never read legacy formats as valid or delete old user files during migration.
