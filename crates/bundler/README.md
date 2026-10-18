# bundler

Library-only retention, exact chunk planning and JavaScript ESM/CJS emission over
the checked owned `SourceGraph`. This is not an unused-report adapter.

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

## Public build session

`build::{BuildSession, BuildOptions, Entry}` joins the existing source cache,
structural resolver, shared graph, effects, exact partition and native linker:

```rust,no_run
use bundler::build::{BuildOptions, BuildSession, Entry};
let mut session = BuildSession::new("project")?;
let mut options = BuildOptions::default();
options.entries.insert("main".into(), Entry {
    path: "src/main.mjs".into(), namespace: true,
});
let result = session.build(&options)?;
let artifacts = &result.product.artifacts;
# Ok::<(), eyre::Report>(())
```

The canonical project root is a bounded dependency/configuration boundary.
All resolved sources must remain inside it. Symlinks (including directory
symlinks) are explicitly rejected by this orchestration API; the lower-level
resolver retains its separate policy. Nearest package metadata determines `.js`
format; explicit extensions override it. Resolution uses the existing resolver's
supported package/tsconfig forms (not every Node resolver feature; e.g. bare
string `exports` is currently rejected). JSON/JSONC bytes and all file names
under the root conservatively invalidate resolution, including package trees,
config ancestry, new files and deleted targets. `.git`, `target`, sidecars and
reserved `.bundler-*` publication directories are excluded. Keep unrelated large
trees outside the project root; this is measured coarse inventory, not a watcher.

Source bytes and effective parser config key in-memory and disk parse reuse.
`Fingerprints` independently identifies parse, resolution, effects, entry roots,
fixed Node runtime and emitter/output inputs. Source caches never contain policy.
The session reports actual extraction misses, memory/disk hits, dependency
resolution calls, linker calls and linker source-check parses separately.
Policy-only changes perform **zero extraction/resolution**, but deliberately
revalidate retained source in the emitter. No-op builds reuse the successful
product and perform zero parsing/resolution/emission. Changed/deleted modules'
transitive prior users come from the shared reverse index. Resolution changes
conservatively invalidate all prior modules. Derived graph/effects/partition/
emission currently rebuild wholesale on any changed key; no fine-grained
later-layer speedup is claimed.

`build` returns `eyre::Result`; failed parsing/resolution/linking/rechecks never
return the prior success. The session retains its last successful product for
later valid builds, not as an error fallback. Cache-write failures are explicit
`warnings`. Sources and config inventory are rechecked before success, not
locked against noncooperating editors. A session has one mutable owner and its
shared products use `Rc` (not cross-thread sharing).

## Transactional publication

```rust,no_run
use bundler::{build::{BuildOptions, BuildSession, Entry}, publish::{Cancellation, current_directory, prune_stale}};
let mut session = BuildSession::new("project")?;
let mut options = BuildOptions::default();
options.entries.insert("main".into(), Entry { path: "main.mjs".into(), namespace: true });
let output = session.root().join(".bundler-output");
let (build, published) = session.build_and_publish(&options, &output, &Cancellation::default())?;
// Run: node <published.entries["main"]>
assert_eq!(current_directory(&output)?, published.directory);
// Only after runtimes using PRIOR generations have stopped:
let cleanup = prune_stale(&output)?;
for warning in cleanup.warnings { eprintln!("{warning}"); }
# Ok::<(), eyre::Report>(())
```

`publish(output, &Artifacts, &Cancellation)` also accepts a successful lower-level
artifact map. It creates unique reserved staging directories, writes/syncs the
complete map, installs an immutable content-identified generation, then atomically
replaces **one** `.bundler-current.json` ownership/current pointer. Callers use
returned generation paths or resolve the current pointer for each fresh runtime;
there is deliberately no set of independently overwritten public launcher files.
No-op publication verifies bytes and preserves artifact and pointer mtimes.
Concurrent publishers use a nonblocking OS file lock: the loser gets an `eyre`
error and may retry; a crashed process releases its lock automatically.

Errors/cancellation before the pointer commit leave the prior generation and
pointer intact. Cancellation is checked before/after building, per staged file
and before commit, not an interrupt inside parsing. Cancellation arriving after
commit cannot roll back success. Publication is atomic visibility, **not**
directory-fsync/power-loss durability or an adversarial-filesystem transaction.
Crash/unwind staging orphans are excluded from watching and never guessed/swept.
Detected cleanup failures are attached to the publication error.

Publishing retains prior immutable generations so an existing runtime's lazy
imports continue working. `prune_stale` is an explicit **quiescent-reader** action.
It removes only validated metadata-listed, hash-unmodified regular files under
known generation names and then empty known parents; never recursive deletion.
Unknown files, modified owned files and symlinks are retained with warnings.
Metadata is bounded and version/owner/path/content-identity checked. Prune errors
may follow partial cleanup of retired generations, never the current generation.
Keep output metadata intact; unowned/colliding generations are not adopted.
The 4 MiB metadata bound can require pruning before many further publications.

Use `publish::is_generated_artifact(path, output)` in watchers. In-project output
for `build_and_publish` must live under `.bundler-*` or `target`, to match source
inventory exclusions. A standalone `publish` can use any explicitly supplied
output directory. No user files beside generated generations are removed.

## CommonJS runtime

Linked opaque JavaScript CJS bodies use Node's native CommonJS loader through
the same synchronous hooks and exact-set payloads. `runtime.mjs::requireModule`
loads canonical bundle-local identities with `createRequire`, not Node's ESM
CommonJS translator (whose private require bypasses these resolution hooks).
Node allocates records before invoking wrappers, binds wrapper `this` to the
initial exports object and returns the current `module.exports`. Recursive
requires see partial exports; deferred sites execute only when called. Failed
factories are removed from the require cache and retry on the next require.
Successful modules execute once. Native ESM proxies carry static named/default
requests; CJS namespace requests/entries use a canonical runtime-key bridge.

### Interop policy

`Artifacts::cjs_proxies` reports canonical proxy tokens and static surfaces.
Reverse typed occurrences supply requested **original** names, including
renamed imports and barrel routes; no CommonJS assignment/export guessing.
Default is always the complete require result; `__esModule` never unwraps it.
Named exports snapshot requested properties after the factory finishes; getters
run once per requested name in sorted order. Missing properties are undefined.
Property mutation is visible through the default object's identity, not through
named snapshots; later `module.exports` replacement affects subsequent require
results but not the proxy's captured default or named values.

A named proxy's native namespace has `default` plus statically requested names.
**Unknown namespace access is conservative, not that finite static list.**
`Artifacts::cjs_namespaces` reports separate native module tokens whose default
exports are canonical CJS namespace bridges. Static `import * as ns`,
`export * as box`, and CJS `loadEntry` use the same bridge object. Its own string
keys are discovered from the actual require result at bridge initialization;
getters forward reads to the captured object, without eager getter invocation.
Thus computed names and `ns[key]` work without source export inference.
The bridge is frozen/null-prototype with a Module tag, not an ESM namespace exotic.
Its `default` is the full original require result even with `__esModule` or a
user `default` property. Keys snapshot once; later property mutations/getters
remain visible, while new keys require `ns.default.newKey` and whole-exports
replacement affects subsequent require only. Star reexports still propagate
statically requested names (snapshots), excluding default.
Factory throws retry through require, but an ESM
proxy evaluation failure is cached by native ESM, as for any failed ESM module.
Proxies and require share the same native CJS cache; no duplicate factories.

### Exact-set proxy integration

`chunks::NodeId::CommonJsProxy { module, name }` participates in ordinary exact
reachability and partitioning: `None` owns initialization/default and `Some(name)`
owns one named snapshot. `NodeId::CommonJsNamespace(path)` owns the conservative
runtime-key bridge. Consumers demand their own names; namespace consumers and
CJS entry facades demand that bridge, not a finite name list. Requires demand the opaque
body/target namespace, not a proxy or eager execution. Manifest synthetic nodes
and chunk dependencies expose proxy→factory and snapshot→initialization edges.
There are no standalone proxy files and no proxy payload duplicated across sets.
Like ESM lexical modules, a canonical proxy assembles all its globally retained
pieces before native compilation. `module_payloads` for a CJS identity includes
factory, named proxy and namespace bridge payloads, separate from logical root load sets.

All chunks share one runtime's native CJS cache and canonical ESM proxy namespace.
`loadEntry` initializes a CJS entry with native require before importing its proxy,
preserving CJS-first mixed cycles. Opaque require of synchronous ESM conservatively
retains the whole target namespace. Native Node owns sync require-of-ESM behavior:
completed/deferred mixed cycles work, while requiring an evaluating ESM in a
cycle throws `ERR_REQUIRE_CYCLE_MODULE`. Require of async ESM is rejected before
emission, even for deferred require sites; use dynamic import instead.
Static ESM routes use canonical file-relative virtual URLs so native synchronous
ESM linking does not encounter an unsupported custom protocol.

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
cannot validate the output. Output is JavaScript ESM/CJS only: no TypeScript or JSX
lowering. The emitter must reject those language modes. No source maps are
generated or advertised; generated offsets are not original-source positions.
Emission removes stale `sourceMappingURL`/`sourceURL` comment directives; it
does not mistake matching string literals or ordinary comments for directives.
Diagnostics for rejected edits refer to original source bytes.

## Single-module emission

`emit::single_module(graph, retention, policy, file_id, source)` returns owned
JavaScript without writing files. Supply a current retention plan; for public
exports, request their checked binding routes or the module namespace when
building retention. Evaluation-only entrypoints need not retain unused exports.
The emitter reparses source, checks owned facts/ranges against the graph,
removes dead statements and applies retention's PURE recipes. Output is reparsed
and semantically checked. Native ESM preserves lexical scopes, hoisting, TDZ,
live bindings and default/alias exports; no wrapper snapshots are introduced.

PURE erasure evaluates callee/reference and arguments in order, consumes spread
iterators, preserves optional-chain completion separately from an undefined
getter value, and composes nested recipes. Fresh arrow parameters avoid capture.
Optional calls/private fields **inside an optional PURE callee chain** are
explicitly rejected; ordinary optional terminal calls are supported.
The current single-module API rejects runtime dependencies (use linked emission),
CJS/TS/JSX modes, and source-policy unsupported syntax such as import.meta.
Source TLA/for-await remains native ESM syntax, including suspension timing.
No externals, implicit TS stripping, source maps or filesystem publication.

## Linked ESM artifacts (Node target)

`link::esm(graph, &ChunkPlan, policy, &BTreeMap<PathBuf, String>)` returns
`Artifacts` containing the complete owned relative `files` map, executable
`entries`, `runtime`, canonical virtual `modules`, exact node-to-payload
`members`, `chunk_artifacts`, physical `module_payloads`, and the unchanged
logical diagnostic `manifest`. It checks freshness and owned source facts,
uses P4 retention/PURE recipes, and rewrites checked static import/re-export
routes. Unused import bindings/re-exports disappear when their statement-level
retention permits it; their dependencies still evaluate. Star ambiguity,
explicit precedence, default exclusion and namespace/live aliases remain ESM
semantics, not object snapshots.

**Target: Node with synchronous `node:module.registerHooks` (22.15+).** Publish
the entire returned map without changing its relative layout. Run an entry
launcher with `node <artifacts.entries[name]>`, or import the generated runtime:

```js
import {loadEntry} from './runtime.mjs';
const namespace = await loadEntry('main');
```

`loadEntry` returns the actual native namespace for an ESM root, the documented
runtime-key bridge for a CJS root, or an ordered array for multiple modules. Request
namespace binding demand when all public exports must remain; evaluation-only
or selected-binding roots intentionally do not retain a complete namespace.
Launchers default-export that namespace object, **not** copied named bindings.
Repeated/concurrent entries and static namespace re-exports share one canonical
native identity per module. Separate published bundle directories have isolated
native identities.

Each logical chunk is one JSON payload of **rewritten** source parts. No
segments are merged, hoisted or duplicated across root sets. Preamble/trivia
and otherwise-unretained static evaluation imports belong to the module-identity
node's existing chunk. The synchronous host hook assembles the retained parts
of one original lexical module, in source order, and returns `format: 'module'`
to Node. Node—not concatenation into a shared scope, `eval`, factories, snapshots
or hand-written binding cells—instantiates and evaluates that real ESM module.
This preserves arbitrary cross-chunk lexical scopes, forward TDZ, function/var
hoisting, cycles, mutation and namespace exotic behavior. Reading a payload
never evaluates another module whose code happens to share that payload.

Physical assembly can synchronously read **all globally retained pieces of a
requested lexical module**, beyond one root's logical diagnostic `load_chunks`.
`module_payloads` reports this distinct physical requirement. It does not change
any root set or constitute a demand-minimal/async loader claim. Payload names
derive from logical identity, not content; transactional/content-addressed
publication is provided by `publish` above. `link::esm` itself writes no files.

Literal source dynamic imports use the same native identities as static imports
and entries. `Artifacts::async_facades` maps each reached source occurrence to
its canonical namespace token; these logical facades add no module wrappers or
evaluation identities. Native import promises preserve microtask scheduling and
deduplicate module evaluation. CJS imports unwrap the existing namespace bridge
with a promise continuation (one extra reaction, not a synchronous require).
Nested sites stay deferred; unreachable sites emit nothing. Successfully read
JSON payloads are cached per runtime, without evaluating co-located modules.
Missing/read/JSON failures retry on subsequent native loads; successfully cached
payload text is immutable for that artifact instance. Compilation errors reject,
and execution errors remain cached by native ESM; rebuilding requires a new
published bundle directory, not in-place repair of cached payloads.
Externals, TS/JSX and other source-policy unsupported forms still fail with
diagnostics. Fetching optimization is not claimed.
This backend is not a browser/standalone-file target.

### Async evaluation and support

Source ESM TLA and top-level for-await remain intact. Node's native module jobs
schedule static dependency SCCs, suspend importers, deduplicate concurrent jobs,
and propagate rejection; there is no competing Rust/JavaScript scheduler.
`ModuleFacts::top_level_await` records only source-local suspension ranges
(nested functions are excluded). The linker derives `Artifacts::async_modules`
by closing those facts over runtime static import/re-export dependencies,
including cycles. Dynamic imports, type-only edges and async function bodies do
not make their consumer async. This derived set is not persisted in parse caches.

Every require of a directly or transitively async ESM target is a link error with
the require's original path/range and an originating TLA path/range, including
deferred/conditional requires. It is never approximated synchronously.
Awaited dynamic cycles retain native deadlock behavior: with no pending work,
Node exits 13 for unsettled TLA; the runtime adds no timers or cycle breaker.

| Source construct | Node 22.15+ linked target |
| --- | --- |
| Literal dynamic ESM import | Native namespace promise, shared evaluation |
| Literal dynamic CJS import | Shared frozen runtime-key bridge; extra promise reaction |
| ESM TLA / top-level for-await | Native host dependency/SCC scheduling and rejection |
| Synchronous require of sync ESM | Native namespace and evaluating-cycle errors |
| Require of async ESM (including static transitive dependencies) | Source diagnostic before emission |
| CJS top-level await / for-await | Parse diagnostic; opaque bodies stay synchronous |
| Nonliteral imports/requires, options/attributes, externals/assets, import.meta, TS/JSX lowering | Explicit unsupported gate; no silent omission |

Bundled-mode Node tests compare all twelve portable cases (fifteen entry executions)
against native baselines **and** fixture stdout contracts. Independent tests cover
cross-root mutation, adversarial cycle TDZ/hoisting, namespace identity and shape,
effect ordering, once-only/concurrent entry loading, bundle-instance isolation,
equal-identity star diamonds, dead-code removal and composed PURE recipes.
These include default/named CJS interop, a self-cycle, dynamic CJS import and TLA.
Independent generated-artifact tests cover async chains, SCC order/hoisting/TDZ,
rejection identity, once-only concurrent entry/dynamic evaluation, unsettled TLA,
and cache-facts → graph → retention → chunks → runtime equivalence. Same-process
concurrent multi-entry CJS cycles/identity have independent differential evidence.

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
modules. P7 also lists exact-chunk CJS proxy initialization/named snapshot and
runtime-key namespace bridge nodes.
External nodes are occurrence-based availability placeholders, not emitted code;
resolve their external specifier through the shared graph.

**Loading code is not evaluating it.** A facade's `evaluation_modules` lists
only initial requested module identities; its `binding_nodes` preserves namespace
or lexical demand. Neither chunks, sorted manifest edges, nor transitive load
sets are a runtime schedule. Distinct module records in one chunk (including
deferred CJS require targets) must retain separate once-only evaluation state.
P6 supplies Node-native ESM rewriting/instantiation (above); P7 supplies CJS
factories/proxies/cache. P8 supplies source dynamic loading and native TLA scheduling.
Only the documented synchronous ESM/CJS subset has emitted-runtime evidence.
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

These analysis recipes are consumed and runtime-tested by `emit` and `link`;
the guarded-callee support restriction above is diagnosed rather than lowered
incorrectly.

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
recompute effects/retention/chunks/manifests/output on policy changes, never
reparse solely for a DSL edit. Plan node/edge indices are local to the graph
instance/revision. Source-local cache identity excludes this entire policy.
The filesystem regression reanalyzes cache-built facts with the source absent
and verifies unchanged sidecar bytes/mtime plus a subsequent warm hit.
