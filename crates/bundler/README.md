# bundler

Library-only effect/retention and exact chunk planning over the checked owned `SourceGraph`.
This is not a JavaScript emitter, runtime, or unused-report adapter.

`retain(graph, evaluation_roots, binding_roots)` keeps module evaluation demand
separate from binding demand. Every runtime static import/re-export demands the
target's evaluation body, even if its bindings are unused. Type-only sites do
not. Effectful statements keep lexical dependencies; function/default-argument/
instance-field bodies do not become top-level effects.

`RetentionPlan` exposes deterministic retained segments/reasons and additional
consumer → dependency edges: static evaluation, module effects and effect order.
Original structural edge indices retain exact occurrence/timing metadata.
Required bodies are **available**, not scheduled eagerly: deferred requires
keep their original call sites/timing. Async boundary edge indices are recorded
but not crossed by retention; `chunks::partition` assigns separate roots. Namespace demand
conservatively retains the entire target, not a guessed property subset.
External edges remain explicit unlinked sites for later output policy.

Construct inputs from `ParsedModule::facts` or decoded cache facts/ranges through
`OwnedSourceFileInput::from_ranges`. Legacy name/usage maps are rejected for
reached modules. Link diagnostics are errors, never success-shaped omissions.
Plans carry the graph revision; recompute after source/resolution changes.
Source order constraints are not a global runtime schedule (especially cycles).
Binding-demand edges also reach their own module-evaluation node, so later root
walks cannot omit effects merely because their initial root is a binding.

Run `cargo test -p bundler`.

## Rewrite backend

`rewrite::{Edit, apply, validate, rename_private}` uses checked original-source
byte ranges and reparses the result with the workspace-pinned OXC parser and
semantic checker. UTF-8 boundaries, overlapping replacements and duplicate
insertions are errors. Private lexical renames use owned binding identities,
not textual matching; object/binding shorthand expands to preserve keys.
Capture and public-binding renames are rejected rather than guessed.
Unedited comments, directives and hashbangs remain byte-for-byte unchanged.

At OXC revision `87f065ebf7cde21d1229322f4c7ee58baca5648e`,
`Codegen::build(&Program)` prints a whole arena-owned AST and its options document
only partial leading-comment preservation. Source edits fit our owned occurrence
spans without reconstructing/rebinding an AST. We therefore use edits, not the
printer/transformer. Reparse actual source on cache hits; source facts alone
cannot validate the output. Output is JavaScript ESM only: no TypeScript or JSX
lowering. The emitter must reject those language modes. No source maps are
generated or advertised; generated offsets are not original-source positions.
Diagnostics for rejected edits refer to original source bytes.

## Root reachability

`chunks::{RootId, AsyncSiteId, RootSeed}` distinguishes caller-named entries from
individual dynamic import occurrences (module path + source-local byte offset).
Identity does not depend on graph file/edge insertion order. Offsets are not
stable across source edits; all graph-local inputs belong to the current revision.
`async_seed` converts a shared occurrence index, not a target path, into a root.
`reference_reachability(graph, retention, policy, roots)` independently flood-fills
each root over synchronous retention edges only and returns sorted path-based
`NodeId` sets, including empty-module identities and explicit external sites.
It rejects stale retention/policy and duplicate or unretained roots. Namespace
binding roots/edges conservatively reach retained target statements; module
evaluation alone does not imply namespace demand. This oracle describes code
availability, not runtime evaluation, especially for deferred synchronous require.

`chunks::partition(graph, entries, policy)` accepts only `RootId::Entry` seeds.
It recomputes existing P4 retention as reachable async sites activate namespace
roots, to a fixed point (including nested sites/cycles). External async sites
stay explicit, never become local evaluation demands. It returns a `ChunkPlan`
containing the final retention, seeds, independent per-root reachability,
`node_roots` (including empty dead sets), and sorted `chunks`. `ChunkId` is the
exact nonempty canonical root set: two live nodes share a logical chunk iff
their sets are equal. No coalescing/hoisting/physical filenames are involved.
`chunk_for` returns None for dead/absent nodes. Call `is_current(graph, policy)`
before using the plan, and recompute when entry seeds change. Graph-local seeds
and retention are revision-local; canonical diagnostic mappings are invariant
under insertion order. This reference implementation prioritizes correctness,
not a bitset/worklist optimization.

## Diagnostic manifests (P6–P8 handoff)

`manifest::diagnostic_manifest(graph, &ChunkPlan, policy)` checks freshness and
returns a deterministic `DiagnosticManifest` (inspect with Rust Debug; no output
files are emitted). `entries` and `async_facades` hold separate facade artifacts,
each with initial `FacadeDemand` and transitive `load_chunks`. Async facades keep
the original consumer/target identities; look up their `AsyncSiteId` in the
owned facts/shared occurrences for source ranges and deferred timing.
An async relationship is **not** a synchronous chunk dependency.

`chunk_dependencies` is the deduplicated consumer→dependency code-availability
graph derived from synchronous retention edges, including namespace binding
expansion. Intra-chunk cycles remain inside their exact chunk; source evaluation
constraints/occurrence metadata remain in `ChunkPlan::retention` and `SourceGraph`.
`synthetic_nodes` explicitly lists entry/async facades outside logical chunks
and module identities inside their existing module-node chunks, even for empty
modules. These records do not manufacture eager evaluation edges or CJS proxies.
External nodes are occurrence-based availability placeholders, not emitted code;
resolve their external specifier through the shared graph.

**Loading code is not evaluating it.** A facade's `evaluation_modules` lists
only initial requested module identities; its `binding_nodes` preserves namespace
or lexical demand. Neither chunks, sorted manifest edges, nor transitive load
sets are a runtime schedule. Distinct module records in one chunk (including
deferred CJS require targets) must retain separate once-only evaluation state.
P6 supplies ESM rewriting/instantiation, P7 supplies factories/proxies/cache,
and P8 supplies dynamic loading/TLA scheduling. No runtime conformance is claimed.
Use manifests only with the same graph instance and after `is_current` succeeds;
recompute the partition/manifest after entry, graph, or effect-policy changes.

## PURE analysis and rewrite contract

`effects::summarize(facts, statement_span, value_demanded)` derives eager reason
sites and `InvocationRewrite` recipes. Only the annotated invocation disappears,
and only if its result is discarded or its initializer/default-export result
is not demanded. Needed results, arguments to other calls, and destructuring
stay conservative. Late binding demand restores the invocation and effect-order
constraints. Deferred function bodies and opaque CJS are not rewritten.

`RetentionPlan::effects` includes summaries for every statement in demanded
modules, even dead statements. An empty summary can drop the unused statement;
a residual-effect summary must apply its recipes, not retain the original call
as an alleged pure statement. Recipes identify the invocation replacement range,
evaluate the original callee/reference once (receiver, computed key and getter
included), then evaluate arguments in order, consuming spreads as iterators.
`ArgumentGuard::OriginalChain` requires preserving the callee's chain-completion
state independently from its value and applying the original optional-call
guard; do not hoist arguments or use an unconditional comma expression.
`StatementValue` recipes discard only an unneeded value, not surrounding effects.
Nested calls whose results feed other expressions stay conservative.
Recipes are a containment tree, not a flat set of independent text edits:
compose contained recipes within their enclosing callee/argument evaluation,
inside its original guard. Never hoist a nested recipe out of an optional call.

These are plans for the P6 emitter, not emitted JavaScript or runtime validation.

## Known-effect policy

`policy::EffectPolicy::parse(patterns)` validates a bounded structural DSL:

- `name(...)`: an unresolved/global identifier callee.
- `global.member(...)`: a terminal member on that exact unresolved/global
  identifier receiver.
- `*.member(...)`: that terminal member on **any** receiver expression, including
  shadowed/aliased receivers. It does not match a standalone method alias.

Names use ASCII identifier syntax (`A-Z`, `a-z`, `_`, `$`, then also digits).
Whitespace around grammar tokens is accepted; the argument wildcard is literally
`...`, not an argument expression. At most 128 patterns of 256 bytes each are
accepted. Unsupported/malformed patterns return indexed `eyre` diagnostics;
there is no JavaScript evaluation, regex or raw-source substring matching.
Patterns describe invocation shape and apply to calls **and construction**.

Static members and computed **string literals** match the same name. Dynamic
keys/template keys are not inferred. Optional calls/members and parenthesized
member references match the same shape without changing execution guards.
Exact global rules exclude lexical shadows (including imports/parameters) and
do not follow receiver/callee aliases or longer receiver chains. Wildcard rules
deliberately include lexical shadows; they add retention, not assumed DOM types.
Nonmatches leave conservative syntax effects/PURE handling unchanged.

`effects::summarize_with_policy` and `retain_with_policy` use only owned facts,
not source text or a parser. Matching rules add deterministic `KnownEffect`
reasons and override PURE hints. Deferred bodies remain deferred; CJS remains
opaque. `with_pure_annotations(false)` conservatively disables annotation
rewrites. Canonical sorting/deduplication makes equivalent policy fingerprints
equal; pattern/annotation-policy changes change the SHA-256 fingerprint.

`RetentionPlan::policy_fingerprint` and `is_current(graph, policy)` check derived
state against both graph revision and effective policy. The caller must
recompute effects/retention/chunks/manifests (and later output) on policy changes, never
reparse solely for a DSL edit. Plan node/edge indices are local to the graph
instance/revision. Source-local cache identity excludes this entire policy.
The filesystem regression reanalyzes cache-built facts with the source absent
and verifies unchanged sidecar bytes/mtime plus a subsequent warm hit.
