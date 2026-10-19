# bundler

Consume owned syntax facts and checked shared graph edges, never UsedTag.
BuildSession is the public orchestration boundary: reuse source-local cache facts,
separate layer fingerprints, inventory config/add/delete changes, and derive
affected users from the existing reverse index. Never return old success on error.
Entry IDs are logical labels. Entry.output_name optionally overrides the
source-relative output path; include it in output invalidation, not root identity.
Renaming canonical files must rewrite requests and artifact routes together.
Later-layer invalidation is deliberately coarse; counters separate source
extraction from generated-output/recipe parsing. Reuse hash-checked cache records
in session emission; public low-level link::esm still validates caller source.
Project roots are bounded/canonical;
symlinks and resolved sources outside the root are rejected.
Product generations are session-local immutable snapshots; never transfer
graph-local handles/plans between products. No-op retains the same generation.
Gate unsupported tsconfig forms rather than silently ignoring extends/references
or allowing nondeterministic overlapping patterns/compatibility-resolver panics.
Publish immutable generations through one atomic ownership pointer; OS locking
rejects competing publishers and spans the whole build_and_publish operation.
Explicitly unlock before closing descriptors, including unwind; forked child
descriptor copies must not keep a completed writer locked. Report release errors.
Never overwrite active generations. Stale cleanup
is explicit after reader quiescence and touches only validated named unmodified
owned files/empty parents, never recursive directory deletion. Source inventory
and watchers exclude reserved .bundler-* paths and source sidecars.
Keep evaluation demand and binding demand distinct. Plans are derived state;
source caches must not contain policy conclusions. Consumer → dependency edges
preserve source order and original deferred/async timing; they are not permission
to execute nested function bodies eagerly.

Rewrites use checked original UTF-8 byte spans and pinned-OXC JS semantic
validation. Never apply overlapping recipe edits independently. Keep native ESM
live bindings/TDZ; no snapshot alias exports. No TS/JSX lowering or source maps.

The symbol linker emits scope-hoisted direct ESM bindings; the rejected
registration/cell/alias/subscriber/generator-dispatch/capture-cycle backend is
deleted. Use native imports/exports across actual output chunks, not per-source
facade families. Public entry/export boundaries constrain output; root sets are
reachability analysis, not unconditional physical partitions or module seams.
Artifact ownership names actual executable files; merged private sources have
no independently executable namespace path in `Artifacts::modules`.
Synchronous evaluation groups merge exclusive chains/body-free connectors only
when source-order completion is preserved; they do not own all source segments.
Keep initialization/await phases and observable namespace/name/read-only
boundaries native, not cells. Logical artifact projections
may overlap or contain several body owners; root sets do not prescribe files.
Native dynamic imports rewrite only literal specifiers to canonical body chunks;
protect complete target namespaces. Keep statically async source modules as
native jobs, preserving original await/for-await and sibling interleaving.
Redirect pure barrels only for equal export-name/canonical-identity maps,
including default; subset matches can widen downstream star exports.
Pure segments can relocate by binding dependencies. Effect retention is distinct
from scheduling and neither is a binding edge. Project through pure/static-import
connectors before pruning to effects; deterministically break order-only SCCs
with visible warnings, never suppress binding/TDZ errors.
Segment ownership overrides evaluation seams; project actual owners into artifact
members. Preserve old export addresses for unchanged consumers and allocate
internal addresses for private bindings/default expressions. Imported writes
must never become writable locals through relocation.
Eager initializer reads and writes constrain lexical components; shared demand
can create independent binding chunks across sources. Split incompatible names
into separate binding chunks, not original-module groups. Namespace demand follows
checked export routes, not private statements; evaluation alone never traverses
namespace edges. Splice empty private evaluation chunks in source order and remap
their no-op artifact members; preserve public/escaped/async namespace identities.
Keep cyclic connector evaluation barriers: revisiting an evaluating job must not
execute its remaining dependencies early. Hoisted availability cannot introduce
new transitive native-initializer activation across a source phase.
Retain effects through shared module-evaluation hubs, not per-segment closures;
hoist static connectors and chain newly entered synchronous static subtrees in
source order. Propagate source TLA over static dependencies; omit async subtree
completion chains. The scalar DAG is a partial order, not an async scheduler.
Cache first-effect projections, including empties, on iterative connector SCCs;
share chain results rather than copying transitive closures per segment.
All roots, including selected binding-only and multi-owner low-level roots, use
native chunks. One ESM binding owner exposes its retained surface; evaluation-only
and multi-owner roots use export-free public wrappers with source-ordered imports.
Entry aliases of one namespace share its canonical path. Empty modules retain
their identities. Preserve supported behavior in independently reviewed increments.
Artifacts report Backend::Native; derive driver layout from it, never filenames.
All portable fixtures require native. The harness retains legacy rejection and
reporting for older binaries, not a production emitter fallback.
Canonical symbols use owned (file, BindingId) identities and checked export routes,
including barrels/redeclarations. Allocated lexical names are not observable source
names or rewrite instructions; boundary values retain typed graph identities.
No hooks, code JSON, source loading/assembly, eval or runtime compilation.
Keep native live bindings, TDZ and const semantics. Only source await/for-await
events suspend evaluation; never wrap every module in async.
Derive async status over static runtime dependencies and reject all requires of
async ESM with original require/TLA spans, including deferred sites.
Do not persist derived async status. Never run a not-yet-evaluated module from a
namespace facade; late completion may only finish newly available binding parts
of an already evaluated module. Externals remain gated.
Each CJS input is one opaque segment bridged by a logical ESM facade; both
participate in ordinary usage analysis/linking/grouping, never a standalone
native .cjs-per-input fallback. Preserve early cache allocation, wrapper
this/exports aliases/current exports and retry after throws.
Chunk helper identifiers must avoid all
source bindings and free references, including in opaque factories.
Pure CJS factories group by availability roots, with one lexical cache per
initializer, never a global ESM table. Facade parts group only at equal consumer
evaluation seams; adjacent sorted snapshots may share a native job. Namespace
bridges keep their canonical default-export seam. Public CJS entries alone need
real CJS wrappers; internal sources have no fake independent module path.
Reverse checked requests supply original names; default is full exports,
__esModule never unwraps, names snapshot once. Namespace keys snapshot once
and reads forward to captured exports. Require-of-ESM uses native evaluation
facades and retains native evaluating-cycle errors.
Physical ownership alone is not fetching correctness. Static execution closure
differs from deferred require availability; never recreate capture cycles.
Never import namespace facades for globally retained but inactive bindings.
Keep real Node import-closure/deleted-async-chunk and late-identity regressions.

PURE recipes remove one invocation, never its callee/reference/argument/iteration
effects. Preserve original optional-chain completion state, not just undefined.
Keep a guarded invocation when lowering would move source await into a generated
arrow; never make that arrow async. Await in the unguarded receiver stays eligible.
Late value demand restores calls; source hints and derived decisions stay apart.

Known-effect patterns are bounded structural add-only rules. Exact globals
exclude lexical shadows; wildcard receivers include them; never infer aliases.
Fingerprints include canonical patterns and annotation policy, not source-cache
identity. Check both graph revision and policy before reusing derived plans.

Root identities use entry IDs or async source occurrences, never target paths.
Canonical diagnostic nodes use paths, not insertion-dependent file numbers.
Root traversal follows synchronous retention constraints only; namespace
binding demand differs from module evaluation. Empty modules keep identities.
Partition only reachable async sites to a fixed point; dead nodes have empty
root sets. Root-set equivalence is analysis, not a prohibition on scope hoisting.
Manifest chunk edges/load sets describe availability, never evaluation order.
Preserve evaluation identity without mandating per-source facade families.
Async links never become sync edges.
