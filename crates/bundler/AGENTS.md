# bundler

Consume owned syntax facts and checked shared graph edges, never UsedTag.
BuildSession is the public orchestration boundary: reuse source-local cache facts,
separate layer fingerprints, inventory config/add/delete changes, and derive
affected users from the existing reverse index. Never return old success on error.
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

Emit one executable registration chunk per exact nonempty root set. Each real
or synthetic node has exactly one physical owner; chunk_artifacts never overlap.
Original bodies live only in chunks, not evaluation/entry/namespace facades.
Relative standard imports/requires are the loader. No hooks, code JSON, source
loading/assembly, custom protocols, eval or runtime compilation.
Binding cells enforce lexical TDZ, const writes and aliases. Native facade
storage is initialized by synchronous binding events, including between
destructuring elements; subscribers are installed through an instantiation
cycle before source dependencies execute. Helper exports stay out of public
namespaces. Do not turn binding events into awaits.
Native ESM jobs still own dependency/SCC scheduling and rejection. Only source
await/for-await events suspend evaluation; never wrap every module in async.
Derive async status over static runtime dependencies and reject all requires of
async ESM with original require/TLA spans, including deferred sites.
Do not persist derived async status. Never run a not-yet-evaluated module from a
namespace facade; late completion may only finish newly available binding parts
of an already evaluated module. Externals remain gated.
Opaque CJS has one assigned factory; the shared runtime allocates its cache
record before call, preserves wrapper this/exports aliases/current exports,
and evicts thrown factories for retry. Chunk helper identifiers must avoid all
source bindings and free references, including in opaque factories.
Proxy initialization, individual named
snapshots and runtime-key bridges each live in their assigned synthetic chunk.
Reverse checked requests supply original names; default is full exports,
__esModule never unwraps, names snapshot once. Namespace keys snapshot once
and reads forward to captured exports. Require-of-ESM uses native evaluation
facades and retains native evaluating-cycle errors.
Physical ownership alone is not fetching correctness. Root-specific hoisted
capture cycles expose actual native namespaces before source bodies run.
Capture follows static execution closure, not deferred require availability.
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
Partition only reachable async sites to a fixed point; chunks are exactly
nonempty canonical root sets. Never merge or hoist. Dead nodes have empty sets.
Manifest chunk edges/load sets describe availability, never evaluation order.
Facades stay outside logical chunk equivalence; module records preserve separate
evaluation identity within shared chunks. Async links never become sync edges.
