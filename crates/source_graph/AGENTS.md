# source_graph

TypeOnlyWiring masks are graph/attempt-local caller-proven connectors. Admission
rejects executable imported references and explicit evaluation-only clause removal.
Both edge construction and value export resolution honor them; native consumers
share runtime_import/export/dependency gates. Facts remain immutable, mask updates
advance revisions, and source/resolution mutation clears derived masks.

Absent-name diagnostics carry typed target/name/namespace evidence; ambiguity
never becomes a recoverable miss. Missing-export leaf traversal follows explicit
renames before stars and terminates cycles without choosing a declaration leaf.
Graph validation/traversal never reads declaration files or repairs source facts.

Edges point consumer → dependency. Forward/reverse indexes refer to the same
occurrence collection. Retain ast_segmenter owned-or-wire storage; do not reconstruct a
second binding IR or put usage tags/effects/runtime scheduling in structure.
Copy reference `async_deferred` per occurrence, never OR consumer/target pairs
or introduce dependencies from the tag. Same-segment reference edges stay intact.
Module nodes preserve empty-file identity; enclosing-function metadata is not
permission to execute a require eagerly.
URL occurrences retain checked resolutions but add no parent evaluation/binding
edges; bundler adds their targets as ordinary independent module entrypoints.
ImportMetaEnv occurrences use normal synchronous import/binding edges to the
build's one synthetic object segment; the graph never reads environment config.
Read `file_facts` Copy views and value iterators; `file_fact_storage` shares the
owner for Ecco/emission. Building graph ranges is not permission to unpack facts.

Lexical binding is independent of TDZ evaluation. Local references and checked
export routes demand all declaration segments of one lexical identity, not only
its representative first declaration. Checked export resolution returns canonical
identity plus barrel route; stars exclude default, explicit
exports win, and ambiguous distinct identities fail. Legacy adapters include
types/return empty on failure; do not use them for executable validation.

Source/resolution epochs invalidate derived handles. Remove tombstones IDs;
recreation appends, never recycles. Patch paths must match input paths.
ID-indexed liveness stays aligned with file storage; checked ID access never
round-trips through the path index. Duplicate construction paths retire prior IDs.
Consumers enumerate `module_ids`, not `0..file_count`, after tombstoning.
Resolver refresh includes prior misses and rebuilds both indexes/cache; coarse
invalidation is the initial correctness contract, not P9 minimal invalidation.

Legacy tracker maps project once into the same edge indexes; missing owned
facts are intentional, not a second nominal IR. Usage walks skip evaluation
edges, keep checked barrel routes, and own their report-only statement ordering.
Cache ranges/facts can construct owned inputs without legacy maps.
`from_parsed` carries extraction/cache provenance; bare `from_ranges` inputs
remain analysis-only. Source patches replace provenance with facts and ranges.
Prepared-resource inputs seal their original path and extraction provenance;
never infer generated parser modes from original resource suffixes.
Effective CJS facts resolve arbitrary value requests to CommonJs(file,name),
not guessed exports. Binding edges preserve requested names and opaque module
demand; runtime snapshot/default policy belongs to bundler proxies.

`dynamic_import_demand` is opt-in derived occurrence metadata, not extra indexed
edges or permission to narrow retention without separate demand roots. Selected `OccurrenceEdge`
binding routes always accompany the original module's async evaluation, never
leaf evaluation alone. Keep requested names separate for per-export roots and
dependency/site identity for activation. Missing properties have no binding;
unknown surfaces, receiver calls and possible exported `then` stay full.
