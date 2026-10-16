# tag_graph

Tracks entry, test, ignored, and type-only usage tags on `source_graph` segments,
derives file tags, and propagates tags to dependencies or their users.

When `follow_type_only` is false, traversal skips explicitly type-only imports,
re-exports, and intra-file references. Mixed type/value references still
propagate runtime tags. Explicit roots retain their tags even when they declare
types; filtering applies to dependency edges, not root identity.

Run `cargo test -p tag_graph` for propagation regressions.
