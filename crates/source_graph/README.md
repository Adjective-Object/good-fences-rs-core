# source_graph

Owns source modules and consumer → dependency structure, independently of usage
tags and bundler effects. `SourceFileInput` is the legacy tracker adapter.
`OwnedSourceFileInput` accepts the existing `ast_segmenter::owned::ModuleFacts`
and per-occurrence resolution results (also reusable from `segment_cache`).

`edges`, `outgoing` and `incoming` expose one authoritative edge collection.
Module nodes preserve empty-file identity. Dependency sites retain source spans,
binding requests, type namespace and sync/async/function-containment metadata.
Containment does not authorize eager execution. Evaluation and binding demand
are distinct; this crate does not implement effects or runtime scheduling.

Run `cargo test -p source_graph`.

`resolve_binding` uses lexical IDs, including forward TDZ-bound references.
`resolve_export` takes an explicit value/type namespace and returns a canonical
binding identity plus all intermediate barrel sites. Stars exclude default,
explicit exports take precedence, equal-identity diamonds merge, and ambiguous/
missing/cycle-only exports fail. `validate_links` exposes construction diagnostics
with source sites. Legacy symbol adapters remain permissive type-inclusive
lookups; executable callers must use the checked linker, not their empty-on-error
compatibility results.

`refresh_resolution` invokes the explicit occurrence resolver for every site,
including prior failures, and returns errors after replacing stale results with
current source-site diagnostics. External/missing sites retain `Unlinked` nodes;
`dependency_target` distinguishes them. Invalidate the resolver before refresh
when its environment changes. No source reparse is needed for resolution edits.

`patch_file`/`patch_owned` reject mismatched paths (rename = remove + add).
`remove_file` tombstones IDs; recreation appends a new ID. Mutations rebuild both
indexes and invalidate export caches. `GraphRevision` separates source and
resolution epochs; `VersionedNode` rejects stale handles even when segment
indices repeat. Paths are supplied identities: the graph never silently
canonicalizes them or merges symlink aliases. Refresh/rebuild is deliberately
coarse, not a claim of minimal P9 invalidation.
