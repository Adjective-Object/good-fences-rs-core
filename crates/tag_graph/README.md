# tag_graph

Tracks entry, test, ignored, and type-only usage tags on `source_graph` segments,
derives file tags, and propagates tags to dependencies or their users.
Both directions traverse `SourceGraph::outgoing`/`incoming`; neither resolves
imports or reconstructs reverse edges. Checked routes keep intermediate barrels
used. Namespace/side-effect/opaque-require dependencies conservatively retain
all target statements, including empty-module identity. Dynamic imports remain
ordinary usage dependencies, not bundler activation or scheduling policy.
Owned-input adapters preserve extraction provenance for downstream emitters;
tag propagation itself remains independent of source bytes and parser policy.

When `follow_type_only` is false, traversal skips explicitly type-only imports,
re-exports, and intra-file references. Mixed type/value references still
propagate runtime tags. Explicit roots retain their tags even when they declare
types; filtering applies to dependency edges, not root identity.

`synchronize` discards tags after structural revisions; propagation calls it
automatically. Call it before setting new direct tags on a mutated graph.
`propagate_tags_for_report` adds caller-supplied predecessor links for the legacy
conservative statement-order report policy, without modifying structural edges.

Run `cargo test -p tag_graph` for propagation regressions.
