# segment_cache

Source-local parse persistence for `ast_segmenter::parse_module`, separate from
the legacy tracker adapter. It stores the existing owned occurrence facts,
module body and ordered statement ranges, not lossy legacy dependency/name sets.
No resolver, usage, effects-policy, graph, absolute path or arena IDs are stored.

P2.1 provides a deterministic versioned CBOR codec. SHA-256 identifies the exact
source bytes and protects the payload against accidental corruption. Identity
includes schema/extractor versions, the pinned OXC revision and effective parser
language/module/definition mode. Current parser defaults are part of the
extractor version; bump it when defaults or extraction/support policy change.

`CacheRecord::from_parsed`, `encode` and `decode` validate all UTF-8 ranges,
source-local IDs, links and ordering. Decode limits input to 16 MiB, 262,144
CBOR items and 32 levels before serde allocates; indefinite lengths and trailing
data are rejected. This is an untrusted cache, not an authenticated artifact.
Diagnostics are not persisted. The codec is owned after parser arena drop.

Run `cargo test -p segment_cache` for codec checks.
