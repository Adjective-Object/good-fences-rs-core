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

Node ESM output stores rewritten source parts in exact-root-set JSON payloads.
Synchronous host hooks assemble one original lexical module before native
compilation; never split its lexical environment or evaluate an entire payload.
Keep physical module_payloads separate from logical manifest load sets. Runtime
URLs include the published bundle directory to isolate separate instances.
loadEntry returns native namespaces; launchers export the namespace object.
Source literal dynamic imports route directly to canonical native identities;
CJS promises unwrap the shared namespace bridge. No facade evaluation copies.
Native ESM jobs own TLA dependency/SCC scheduling and rejection. Derive async
status only over runtime static dependencies; reject all requires of async ESM
with original require/TLA spans, including deferred sites. Never persist this
derived status or add an eager/manual scheduler. Externals remain gated.
P7 CJS uses native createRequire + synchronous hooks, not the ESM CJS
translator's separate require path. Native cache owns allocation-before-call,
partial cycles, wrapper this, current exports and eviction/retry after throws.
Proxy initialization and named snapshots are exact-set synthetic nodes; never
hide them in facade files. Reverse checked requests supply original names.
Default captures full exports ignoring __esModule; names snapshot once. Unknown
namespace demands use a separate exact-set runtime-key bridge, not finite names.
Keys snapshot at initialization; property reads forward live to captured exports.
Static ESM namespaces remain native; CJS bridges are frozen/null-prototype objects.
Require-of-ESM retains namespace; native evaluating-ESM cycles still throw.

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
