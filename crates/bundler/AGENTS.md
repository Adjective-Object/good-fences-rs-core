# bundler

Consume owned syntax facts and checked shared graph edges, never UsedTag.
Keep evaluation demand and binding demand distinct. Plans are derived state;
source caches must not contain policy conclusions. Consumer → dependency edges
preserve source order and original deferred/async timing; they are not permission
to execute nested function bodies eagerly. No emitted-runtime correctness claim.

PURE recipes remove one invocation, never its callee/reference/argument/iteration
effects. Preserve original optional-chain completion state, not just undefined.
Late value demand restores calls; source hints and derived decisions stay apart.
