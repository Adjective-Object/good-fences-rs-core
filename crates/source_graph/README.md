# source_graph

Owns source modules and consumer → dependency structure, independently of usage
tags and bundler effects. `SourceFileInput` is the legacy tracker adapter.
`OwnedSourceFileInput` accepts the existing `ast_segmenter::owned::ModuleFacts`
and per-occurrence resolution results (also reusable from `segment_cache`).

`edges`, `outgoing` and `incoming` expose one authoritative edge collection.
Legacy tracker inputs project their lossy maps into this collection once at
construction/mutation, including checked barrel routes. They have no lexical
occurrence facts: spans are statement-level, absent targets remain `Unlinked`,
and import lookups retain the tracker's type-inclusive compatibility policy.
Use owned inputs for executable validation and precise occurrence metadata.
`OwnedSourceFileInput::from_ranges` accepts decoded cache ranges/facts without
reconstructing legacy dependency or variable maps.
Module nodes preserve empty-file identity.
`module_ids` enumerates live IDs including empty modules, skipping tombstones;
`file_count` is a count, not an upper bound on IDs after removals.
Dependency sites retain source spans, binding requests, type namespace and
sync/async/function-containment metadata.
Containment does not authorize eager execution. Evaluation and binding demand
are distinct; this crate does not implement effects or runtime scheduling.

Run `cargo test -p source_graph`.

`file_facts` exposes owned syntax records (None for legacy/tombstoned modules).
`outgoing_indexed` exposes shared occurrence indices for derived plans; indices
are revision-local, not persistent IDs. The `bundler` library consumes these
without adding effects policy or usage tags to the shared graph.

`resolve_binding` uses lexical IDs, including forward TDZ-bound references.
It returns a representative declaration; local-reference edges and checked export
routes retain every declaration of that identity, including later `var`
initializers, without crossing lexical shadows.
`resolve_export` takes an explicit value/type namespace and returns a canonical
binding identity plus all intermediate barrel sites. Stars exclude default,
explicit exports take precedence, equal-identity diamonds merge, and ambiguous/
missing/cycle-only exports fail. `validate_links` exposes construction diagnostics
with source sites. Opaque CommonJS value requests resolve to
`ExportIdentity::CommonJs(file, requested_name)` and a module/body demand, not
guessed source exports. Type requests do not gain this dynamic fallback.
Legacy symbol adapters remain permissive type-inclusive
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

Unused analysis shares the immutable graph with reports through `Arc`; the
legacy export cache is synchronized so parallel report readers remain safe.
Usage tags and conservative statement-order reporting stay outside this crate.
