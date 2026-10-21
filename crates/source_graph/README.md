# source_graph

Owns source modules and consumer → dependency structure, independently of usage
tags and bundler effects. `SourceFileInput` is the legacy tracker adapter.
`OwnedSourceFileInput` retains `ast_segmenter::views::FactsStorage`
and per-occurrence resolution results (also reusable from `segment_cache`).

`edges`, `outgoing` and `incoming` expose one authoritative edge collection.
Legacy tracker inputs project their lossy maps into this collection once at
construction/mutation, including checked barrel routes. They have no lexical
occurrence facts: spans are statement-level, absent targets remain `Unlinked`,
and import lookups retain the tracker's type-inclusive compatibility policy.
Use owned inputs for executable validation and precise occurrence metadata.
`OwnedSourceFileInput::from_parsed` preserves extraction provenance, including
checked cache records from `CacheRecord::to_parsed(source)`, without rebuilding
legacy dependency or variable maps. `file_provenance` exposes the source/config
integrity evidence used by parse-free emission. `from_ranges` remains available
for analysis-only inputs; without provenance these cannot be emitted. Source
patches replace provenance and invalidate derived handles, just like facts.
`from_prepared_resource` additionally seals the original resource path with its
extraction provenance. `file_prepared_resource` lets emission check that identity
and reuse the generated parser mode without guessing from the input extension.
Module nodes preserve empty-file identity.
`module_ids` enumerates live IDs including empty modules, skipping tombstones;
`file_count` is a count, not an upper bound on IDs after removals.
Dependency sites retain source spans, binding requests, type namespace and
sync/async/function-containment metadata.
Containment does not authorize eager execution. Evaluation and binding demand
are distinct; this crate does not implement effects or runtime scheduling.
Local-reference occurrences copy `async_deferred` from owned syntax facts,
including existing same-segment edges. Mixed async/sync references to one target
stay separate in both indexes and retain their original sites; the flag neither
adds dependencies nor changes activation. Dependency and legacy edges remain
false. Derived consumers use `source_edge` occurrence indices, not pairwise ORs.

`DependencyTarget::Context` maps captured relative request spellings to source
paths. It expands one real dynamic-import/require occurrence into candidate
edges, retaining its original ID, span and activation. Aliases of one target
share an edge. Checked linking rejects mismatched patterns/targets and missing
candidate modules; an empty closure is valid and fails only when requested.

Run `cargo test -p source_graph`.

`file_facts` returns Copy owned-or-wire syntax views (None for legacy/tombstoned
modules). Collections expose value iterators and `get`, not owned slice layout.
`file_fact_storage` retains the immutable owner for emission/Ecco adapters.
`from_parsed` consumes `ParsedModule::into_parts` without cloning or unpacking the
fact tree; only graph-owned statement ranges are constructed for wire inputs.
Graph-derived identities and indexes remain separate from storage.
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
with source sites. `resolve_optional_export` distinguishes absent namespace
properties from ambiguity, which remains an error. Opaque CommonJS value requests resolve to
`ExportIdentity::CommonJs(file, requested_name)` and a module/body demand, not
guessed source exports. Type requests do not gain this dynamic fallback.
Legacy symbol adapters remain permissive type-inclusive
lookups; executable callers must use the checked linker, not their empty-on-error
compatibility results.
Unknown external star surfaces cannot prove missing/ambiguous names at build
time. Distinct candidate identities involving externals resolve to an explicit
`ExportIdentity::Native(module, name)` boundary with all candidate routes; the
native loader validates that binding. Fully known internal ambiguities still
fail. `has_external_star_exports` identifies transitive open namespace surfaces.

`dynamic_import_demand(source_edge)` derives opt-in `DynamicImportDemand` metadata
for a runtime dynamic-import edge at the current revision. `evaluation` is an
asynchronous evaluation edge to the **original** module (including barrels);
every selected export requires that same occurrence's evaluation obligation.
`bindings: Some(routes)` uses ordinary `OccurrenceEdge` binding routes with the
requested `ImportedName::Name`, original dependency ID, site and containment.
Group these by requested name for future per-export roots, never canonical leaf
identity; keep activation keyed by the original occurrence. Missing properties
have no binding route or static diagnostic, and `Some([])` is evaluation-only.
`bindings: None` requires the original full namespace: unknown/escaping/reflected
observations, receiver calls, possible exported `then` (promise assimilation),
CommonJS, external/native/namespace-valued routes and contexts are conservative.
No callability or receiver-safety proof is inferred. Observations remain borrowed
owned-or-wire views, not symbol aggregation. These demands do not enter the edge
indexes or change today's retention/chunk planner; consumers must explicitly
support separate binding roots and original-module evaluation before using them.

`refresh_resolution` invokes the explicit occurrence resolver for every site,
including prior failures, and returns errors after replacing stale results with
current source-site diagnostics. External/missing sites retain `Unlinked` nodes;
`dependency_target` distinguishes them. Invalidate the resolver before refresh
when its environment changes. No source reparse is needed for resolution edits.
Context graphs require rebuilding owned resolutions with the caller's bounded
inventory instead; `refresh_resolution` rejects them before mutating the graph.

`patch_file`/`patch_owned` reject mismatched paths (rename = remove + add).
`remove_file` tombstones IDs; recreation appends a new ID. Mutations rebuild both
indexes and invalidate export caches. Checked ID access uses an aligned liveness
vector, not path hashing; removed IDs remain inaccessible after recreation.
`GraphRevision` separates source and resolution epochs; `VersionedNode` rejects
stale handles even when segment indices repeat. Paths are supplied identities:
the graph never silently
canonicalizes them or merges symlink aliases. Refresh/rebuild is deliberately
coarse, not a claim of minimal P9 invalidation.

Unused analysis shares the immutable graph with reports through `Arc`; the
legacy export cache is synchronized so parallel report readers remain safe.
Usage tags and conservative statement-order reporting stay outside this crate.
