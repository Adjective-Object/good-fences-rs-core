# source_graph

Edges point consumer → dependency. Forward/reverse indexes refer to the same
occurrence collection. Reuse ast_segmenter owned facts; do not reconstruct a
second binding IR or put usage tags/effects/runtime scheduling in structure.
Module nodes preserve empty-file identity; enclosing-function metadata is not
permission to execute a require eagerly.

Lexical binding is independent of TDZ evaluation. Checked export resolution
returns canonical identity plus barrel route; stars exclude default, explicit
exports win, and ambiguous distinct identities fail. Legacy adapters include
types/return empty on failure; do not use them for executable validation.

Source/resolution epochs invalidate derived handles. Remove tombstones IDs;
recreation appends, never recycles. Patch paths must match input paths. Resolver
refresh includes prior misses and rebuilds both indexes/cache; coarse
invalidation is the initial correctness contract, not P9 minimal invalidation.

Legacy tracker maps project once into the same edge indexes; missing owned
facts are intentional, not a second nominal IR. Usage walks skip evaluation
edges, keep checked barrel routes, and own their report-only statement ordering.
Cache ranges/facts can construct owned inputs without legacy maps.
