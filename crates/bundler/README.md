# bundler

Build and publication errors retain `eyre` cause chains with project/module paths
and stage/IO context. Worker panic reports retain string payloads and all workers
are joined. Display the full chain with `{error:#}`; `to_string()` shows only the
outer context. Diagnostic paths/requests are not added to duration traces.
High-level `bundler::progress` tracing events mark build/publication phases and
report numeric preparation/cache counts at most once per second. The library
installs no subscriber; bundler-cli reports these events to stderr independently
of its optional duration trace.
The separate `bundler::progress::discovery` events sample root-relative filenames
and traversed directories for stderr feedback, at most once per second per kind;
these path-bearing events are excluded from duration traces.

## Replacement direction and current status

The [fixture semantic contract](../../fixtures/bundler/README.md#permitted-execution-differences)
lists permitted execution differences and the observations adversarial tests
must preserve. TDZ-error preservation is waived; generated suspension and new
load failures require explicit async-boundary opt-in.

Graph linking, effect policy, Ecco registration, execution constraints and native
recipe rendering consume owned-or-wire AST views. A byte-backed graph retains
verified owners, including Ecco's nested storage, without unpacking extraction
trees. Derived plans remain owned. Production `BuildSession` sidecars use typed
`.seg.fb` records (schema 18/extractor 27); parse fingerprints include both active
versions. Legacy `.seg.cbor` and both staging formats are excluded from inventory,
but never loaded as current facts or deleted by the migration.

Preparation stages only source identities and settled resolutions, not owning
graph inputs or a second source-text map. New products construct graph inputs
once from checked session records after the no-op check; linking borrows the
settled source owners. Failed attempts retain useful prepared sources without
replacing the last product. Returned products own their graph records independently
of the session; no-op builds retain the existing product.

Computed dynamic imports without a static relative prefix recursively capture
all files in the importing file's directory and descendants, through the checked
inventory and normal script/JSON/CSS/asset policy. Parent/sibling directories and
reserved output/cache paths are excluded. A source/site warning about broad
capture and bundle growth appears in build warnings and stderr progress on cold,
warm and no-op builds. Relative template/concatenation prefixes remain narrower.
Attached boolean `webpackIgnore: true` block or line comments leave literal and
computed imports native, without resolving or capturing dependencies; arguments
still evaluate normally. JSDoc/unrelated comments and string contents do not
enable ignoring, and explicit false follows normal bundling.

### Package-local resolution caches

Production `BuildSession` resolves actual literal/context/resource requests through
independent `.bundler-resolution-v1-<policy-sha256>.fb` package sidecars. The policy
key covers effective resolver options, externals, platform and implementation,
not inventory contents. Package keys include physical instance, original mutable
manifest bytes and mapped lockfile named dependency fingerprints. Importer paths,
request spelling and import/require mode remain separate request identities.

The pinned `aube-lockfile` 2.8.1 graph is parsed once per build and shared by script
and Sass policies; selection and source bytes are checked around parsing and at
return. Canonical workspace paths and explicit installed locators map to graph
records. Berry installations without locators, incomplete graphs, missing or
unreadable/unparseable lockfiles remain correct through conservative
manifest/physical identities and full actual-request proofs. No peer provider is inferred and the
lockfile never changes installed-tree resolution. Npm/shrinkwrap and Classic
retain stage one's documented coarse raw-lockfile hashes.

Lockfile fingerprints use the configured execution worker limit (one runs inline).
A coordinator tracks outstanding dependency components and dispatches ready jobs
through a bounded channel; completions immediately unlock their dependents.
Cycles are single SCC jobs, preserving the existing hash bytes and mutable/
incomplete propagation regardless of completion order. Hashing and per-member
fingerprints run on workers; only the coordinator stores results and emits
`bundler::progress::lockfile` completion counts.

Disk hits validate borrowed original positive/negative probes directly, without
unpacking proof trees. Fresh resolutions retain their original proofs too.
Both paths are rechecked before success, including no-op returns. A nearer file,
changed package export or link target cannot authorize stale reuse. Unportable
external footprints fall back to fresh scoped resolution with a diagnostic.
Cache corruption/read/write failures are nonfatal and explicit; read-only mode
does not publish and disabled mode does not read/write these files.

Writes preserve other still-valid requests in one complete atomic policy
aggregate, after final build checks. `Counters.resolutions` measures actual scoped
resolver calls; `resolution_cache_hits`/`resolution_cache_misses` expose request
reuse separately from extraction hits. The session's source-resolution map is
derived graph-edge state only. Graph/output fingerprints depend on actual source
dependency facts, resolved edges and resolver policy: unrelated JSON/config or
disconnected Berry lockfile edits do not change no-op product generations.

Each current build has one shared row per resolver request, carrying its original
memo and demand state. Package publication indexes share those rows rather than
copying request keys or proofs. Only validated demanded requests are promoted to
session memory; they remain reusable after failed builds, while speculative tails
are discarded even on early errors. Retained requests revalidate their original
fresh or borrowed disk proofs on first use in each filesystem epoch.

Attempt-local request keys, native path/request/target payloads and drop-free
rows use a lexical scratch arena. Each participating worker acquires one
`bumpalo-herd` Member through attempt-local thread storage, not per allocation.
Native path units round-trip without Unicode conversion. Proof/Arc owners,
locks, errors and map buckets retain normal RAII cleanup. Precise promotion
copies only demanded key/target payloads into independently owned session
memory and shares the original compact proofs. Neither retained caches nor
returned products pin the scratch arena or its filesystem epoch.

Inventory admission, manifest reads and script/Sass resolution share one
build-scoped filesystem snapshot, including speculative workers' reads.
Speculative inventories still retain private observations/admission state; only
demanded observations enter final validation. Repeated positive/negative paths
and ancestor link prefixes are memory-only hits. Persisted proofs compare against
current-epoch observations without restatting shared inputs. Final checks use new snapshots and compare the original
observations, including on no-op builds. Cache encoding/aggregation/publication
reuse the validated boundary snapshot, rather than repeatedly reopening every
request's inputs; post-publication checks remain fresh. Lockfile identities
never substitute for the physical installation observations.

Precise input admission remains demand-driven. An upfront multi-root walk supplies
speculative script candidates from entrypoint packages and their local runtime,
optional and selected-peer dependency closure, excluding dev dependencies.
Installed package manifests can connect that closure, but `node_modules` sources,
unselected nested packages, reserved directories and ignored files are not walked.
Package-closure probes run on bounded workers. A single bounded `jwalk` pool
enumerates the selected trees, applying inherited `.gitignore`/`.ignore` rules
on workers without per-file canonicalization or source preprocessing.
The walk does not preprocess sources or authorize imports. Normal preparation
still applies to candidate TS/JSX files. Unreachable errors, observations and
requests never enter successful build validation, diagnostics or resolution-cache
publication. Finite contexts retain their precise candidate checks before success.

### Upstream JavaScript source maps

`BuildOptions.output.source_maps` defaults to `false`. Enabling it emits external
JavaScript and stylesheet maps (see Sass inputs below), with JavaScript
composition using each script's last parser-identified `sourceMappingURL` comment.
Inline JSON data URLs (base64 or percent-encoded) and linked local maps are
supported; linked files must belong to the checked canonical inventory, including
explicit linked trees. Remote
links, missing files, malformed maps and unsupported encodings are errors, not
fallbacks. No original source or network URL is fetched. Disabled maps discover
directive spans during extraction but do not load or validate map contents.

Original names and nullable `sourcesContent` survive composition. `sourceRoot`
and source paths resolve from the linked map location (or script location for
inline maps), producing absolute source URLs in merged output. Sparse/unmapped
upstream segments and generated wrappers remain unmapped. Only JavaScript
script inputs consume these upstream directives; prepared resource wrappers
have no original spans.

Parsed upstream metadata uses a separately checksummed, typed FlatBuffer at
`<script>.upstream.seg.fb`. It binds exact script/map bytes and map location;
verified cache hits read borrowed tables without parsing JSON or unpacking
records. Linked map bytes are rechecked on every build and before no-op returns.
Maps-only edits invalidate output and reverse users, not JavaScript extraction.
Enabling/disabling maps is likewise an output-only change. These sidecars are
separate from the production AST cache's typed FlatBuffer records.

### TypeScript and JSX inputs

`BuildSession` precompiles `.ts`, `.mts`, `.cts`, `.tsx`, `.jsx` and their explicit
module JSX variants `.mtsx`/`.mjsx`/`.ctsx`/`.cjsx` with the
pinned Oxc 0.155.0 transformer/code generator before extraction and dependency resolution.
The JavaScript pipeline requires Rust 1.97 or newer; the repository's pinned
nightly satisfies this requirement.
Type-only dependencies disappear before resolution; runtime TypeScript constructs
and JSX become JavaScript. Declaration files are not executable inputs.
The generated parser mode preserves the input's module format, including
CommonJS `.cts` inputs. Default entry names use JavaScript `.mjs`/`.cjs` extensions.
Static import/export syntax or `import.meta` disambiguates CommonJS-default
`.js`/`.jsx`/`.ts`/`.tsx` as ESM; dynamic import alone does not. The effective mode
and detection policy are cached, so hits never repeat syntax classification.
Explicit m/c extensions always win, including JSX variants; ESM syntax in
`.cjs`/`.cjsx` is an error. The pinned transformer does not convert ESM imports/
exports to CommonJS in `.cts`/`.ctsx`; type-only imports are still erased.

JSX uses the automatic production runtime with the default `react` import source;
its injected runtime requests resolve through the ordinary checked package graph.
No runtime package is bundled implicitly, no tsconfig is discovered, and no type
checking or compatibility-target downleveling is performed.

With `output.source_maps: true`, Oxc compiler maps travel alongside the prepared
JavaScript and compose through any source map referenced by the original input.
Output maps retain original TypeScript/JSX URLs, contents and UTF-16 coordinates,
not generated JavaScript coordinates. Disabled maps skip compiler-map generation.
Warm builds reuse prepared JavaScript; enabling maps may rerun compilation but
does not reextract unchanged generated JavaScript. Type-only edits still refresh
map contents/positions even when generated JavaScript is unchanged.
Low-level emission APIs continue to accept only already-prepared JavaScript.

The symbol linker uses scope-hoisted direct ESM bindings and native
imports/exports only across actual output chunks. Root sets remain reachability
analysis, not mandatory physical partitions. Public entry/export boundaries
constrain output; pure segments may move according to binding dependencies.
Sparse mandatory execution components and consumer demand choose physical owners;
pure siblings and shared declarations co-locate without whole-module absorption.
Native SCC evaluation remains entrance-dependent, not permission to concatenate.
Native jobs remain where required for
public/escaped namespaces, top-level-await phases, imported writes, observable
names or CommonJS activation. Merging never hoists a later shared dependency
ahead of an earlier initializer.
Each CJS input contributes one opaque segment plus a logical ESM facade, both
subject to normal usage/linking/grouping, not a native `.cjs`-per-input fallback.

The registration/cell/alias/subscriber/generator-dispatch/capture-cycle backend
is **deleted**, including its runtime and fingerprints. All linked output is
native; there is no legacy-backend fallback or backend feature flag.
The internal `symbols::Symbols::link` builds checked canonical
`(file, BindingId)` definitions and import/export routes using the shared graph
resolver. Native emission reuses that plan, preserving intermediate barrels and
every retained var declaration.
Source imports and references carry binding-availability, execution/activation,
native-external-validation and exact-public-surface obligations; they do not
prescribe physical module seams. `RetentionPlan` and canonical symbols feed the
segment owner solver. Source privacy never adds a placement constraint.

### Precise native dynamic imports

`chunks::partition_precise` consumes checked `SourceGraph::dynamic_import_demand`
routes. `RootId::AsyncExport { site, name }` identifies each requested export
independently, even when two names share a canonical binding. `AsyncSiteId` still
identifies activation, not a union of selected bindings. Every export root seeds
the **original import module's evaluation** plus its checked binding route;
barrel/sibling effects therefore belong to every selected root. Pure route
connectors do not demand their other import/export specifiers. Full namespaces
and public roots still retain their entire surface, including named CommonJS
snapshots resolved through local export connectors whose import declarations
are pruned. These requests follow each root's namespace-binding provenance,
not module evaluation alone. Missing properties need
evaluation only; unsafe observations, possible `then` exports, receiver calls,
contexts, CommonJS and external surfaces retain the graph's conservative demand.

`ChunkPlan::async_activations` maps each original occurrence to its independent
roots. `DiagnosticManifest::async_facades` keeps one activation projection:
`target` is the original import target, `demand_roots` preserves each root's
demand/load set, and `facade` is their availability union, not another root.
Nested occurrences are discovered only from reached consumers to a fixed point.
Generated private-function boundaries retain their existing separate contract.

`partition`, `partition_with_options` and `BuildSession` use precise planning by
default. `partition_conservative` remains the namespace reference; both plans
can emit. After physical placement, typed `Connections` gives each selected
occurrence a native ESM projection: it evaluates the original target and reexports
only its requested canonical live bindings. No private transport names escape.
Both unchanged source recipes and reconstructed bodies route imports by source
occurrence, not target. Missing properties have evaluation-only projections.
Selected pure components group by their exact demand masks, independently of
occurrence projections, and small-library clustering cannot merge these owners.
Certification checks the complete initializer closure, so unrelated full-namespace
islands cannot move selected targets behind guards that their projections bypass.
Shared helpers retain union masks; original effect/TLA/failure jobs remain shared.
Full/public consumers still use complete native namespaces. No source-loading
runtime or mutable namespace cells are introduced.
CommonJS snapshot recovery follows planned facade demands rather than retained
multi-export statements; native external seams prune direct and local export
clauses to the same retained surface.

### Lazy-module certification and graph overlays

Disabled by default. `OutputOptions::lazy_runtimes: Vec<lazy_modules::LazyRuntime>`
registers three exact `ConstructorExport { module, export }` identities for the
module, import wrapper and action wrapper. Module paths must be graph identities:
canonical absolute source paths in `BuildSession`, or the exact paths supplied
to a low-level graph. There are no builtin package/path or identifier-name rules.
Import aliases, reexports and simple immutable const aliases resolve to canonical
linked bindings or immutable default-value expressions. Source homonyms and
shadows are not authorization. Mutable/reassigned registered constructor variables and registrations assigning
conflicting roles to one canonical constructor are explicit configuration errors.
Missing modules/exports and noncanonical graph identities also fail explicitly,
including in external-native fallback mode; unsupported source instances still
fall back unchanged.

Registration explicitly trusts the library contract for
`LazyModule(callback, options)` and `LazyImport`/`LazyAction(module, getter, ...args)`:
module allocation may be eliminated/synthesized and warming/loaded state may
become split-specific. Absent options must mean no initializer. Plain own
option properties without an initializer must not acquire one implicitly.
Any possible initializer rejects the instance, because it runs once after load
in the original library. The actual wrapper constructors, methods, optional
governance, cached selection, module loaded-state reads, action bundle-time
options and action-call arguments must remain library behavior, not promise
substitutes.

`lazy_modules::certify` runs before retention in `partition_with_options` and
public builds. It returns `ChunkPlan::lazy_modules`, a revision/policy/configuration
stamped `Certification`, with borrowed read access through `modules()`.
`is_current_with_options` also checks runtime configuration. Certification
requires a literal local ESM callback, a simple dotted selector, checked export
routes and no namespace/unknown/CommonJS surface or possible namespace `then`.
Every runtime reference to a module binding must be a recognized wrapper's module
argument. An exported module, escape, direct operation, unsupported selector or
unknown wrapper rejects that entire instance, not only the offending use.
Selectors constructed inside their own module initializer (including its option
closures) also leave the entire instance original. Graphs using external-native
linking retain original instances: that separate clause-preserving emission mode
does not yet support lazy lexical reconstruction.
Nested statement-value initializers lacking exact result-binding boundary
evidence (for example sequences or annotated TS declarations) also retain the
original instance rather than guessing its liveness. Required-value conditional
uses keep their original construction sites.

Options must be absent or have checked source-local plain literal-property
evidence. Initializer/prototype keys, computed/spread/accessor/method properties
and arbitrary expressions fall back. The initial evidence subset also excludes
parenthesized option objects, escaped keys and regexp/division-bearing values.
Effectful values and captures are **not** assumed pure: original option and
wrapper-argument slots retain operation roots, reference/effect/invocation indices
and dependency identities. They must be evaluated once at their original site,
not duplicated across synthesized instances.

Certificates include original declaration/selector identities, canonical
constructors, the real dynamic occurrence, original-target evaluation and each
selected canonical export/route. Before retention, a build-local overlay consumes
the module construction's invocation contribution, its broad callback import,
and each selector's reference to the removed module binding. No cached facts are
mutated. Trusted wrapper constructions are removable only when their values are
unused; residual argument effects and all genuine constructor/getter/additional
argument dependencies remain. Invocation summaries do not make enclosing
statements pure. Nested local initializer results with reads outside their
construction are conservatively live even without a segment value demand;
the actual initialized result binding comes from checked AST declaration evidence.
Captured reads count regardless of textual order; unrelated locals and getter
parameters never make a discarded selector live.
Symbol definitions and execution accesses exclude the removed
module binding; scheduling consumes only the proven invocation contributions.
Original options are retained once through a `PreservedSlot`
obligation, including closure captures; they are never copied into selectors.

Every retained selector independently activates an `AsyncExport` root with its
checked selected binding route and **original barrel evaluation**. Discovery
iterates to a fixed point, including nested instances but excluding dead
selectors. Shared effects/helpers retain ordinary union masks, not an aggregated
symbol root. `AsyncActivation::origin` distinguishes source imports from
`AsyncOrigin::Lazy { declaration, module_candidate, selector }`; lazy offsets
identify selector constructions, never invented source dependency IDs.
Diagnostic async facades report the selector consumer and original import target.

`link::native_placement(graph, plan, policy, sources, options)` solves native
ownership without emitting or reparsing source. Its immutable `NativePlacement`
exposes actual `NativeOwner` assignments, owner schedules, canonical cut-binding
inputs and `LazyWiring` requests. These are graph/plan-generation-local analysis
results, not artifact filenames or reusable source-cache records.
Each request identifies the final selector owner, original module/candidate and
selector, exact selected roots/canonical destination owner, original barrel
evaluation, generated constructor symbol/owner and optional `PreservedOptionsValue`
at its single original owner/span. Post-placement connections can consume these
typed inputs without inspecting rendered code.

Registered wrapper allocations remain real value initializers, not unconditional
module effects that retain every sibling selector. Placement uses their residual
argument obligations and entrance-specific binding projections; unknown argument
execution, forced effects and original initialization/throw fences remain binding.
Generated constructor reads participate in symbol linking, owner-local naming,
activation checks and scheduling at the selector instead of at the removed
module declaration. Preserved option values remain genuine once-only inputs.
Eligible selectors declared in one source can therefore belong to separate
consumer owners, with independent implementation owners and ordinary shared
helper/effect masks. Conservative constraints may still prevent relocation.

Native emission now consumes those typed placement requests in `Connections`.
Synthesized instances are keyed by **final source chunkgroup × original module
identity × destination projection**. Compatible selectors in one group share
their destination's exact selected-export projection; different source groups,
original modules or destinations have independent loaded/warming state. A
projection evaluates the original barrel and reexports only demanded selected
native bindings, never the original broad namespace.

Each instance uses a collision-safe owner-local binding initialized at the
retained selector's original module-argument slot. Nested/conditional selectors
therefore do not run wrappers eagerly. A collision-safe owner-scope constructor
reader prevents nested parameters from capturing injected reads, without
snapshotting the constructor or moving its initialization/read time.
Only that argument is rewritten: wrapper
class, getter expression/name, extra arguments, methods, caches, errors, retries
and metrics remain the registered library's actual code. Unused constructions
discard their consumed module/getter inputs but preserve genuine extra-argument
evaluation in order.

The original options slot initializes one immutable generated binding at its
original scheduled phase, even when all selectors are unused. Cross-owner
consumers receive real native read-only imports of that value, not copies of the
expression; private transport names never enter public namespace projections.
Source initialization/throw checks still validate the final reconstructed graph.
No global table, synthetic library runtime, source loading or reparse is used.
Use `link::esm_with_options` with the same registrations as partitioning; plain
`esm`/`esm_with_execution` require the default empty configuration.
`deferred_esm` declines certified lazy plans rather than partially reconstructing
them. Unconfigured and unsupported-instance builds retain original behavior.
Configuration-only changes invalidate derived output, reuse source facts and
participate in no-op detection; registration ordering/duplicates are immaterial.

The bounded bundler CLI accepts `fixtureConfig.output.lazy_runtimes` as a list
of at most 16 `{module, import, action}` objects, each role an exact
`{module: "root-relative.mjs", export: "Name"}`. Paths must be portable relative
files, canonicalized inside the build root; names are nonempty/control-free and
at most 256 UTF-8 bytes. Unknown fields are errors; the whole fixture config is
bounded to 16 KiB. `fixtureConfig.cache` is only `"read-write"` (default) or
`"disabled"`. This is not general `BuildOptions` deserialization.
Authored `lazy-selectors-placement` and `lazy-runtime-contract` fixtures exercise
the public source/output pipeline. Physical tests use declaration counts,
`segmentMembers` owners/exact roots and native edges at byte targets 0/10000,
plus disabled/cold/warm equivalence. Real source-phase constraints, including
local constructor-alias initialization, can still prevent otherwise desired
selector relocation.

### Native connections

After placement, internal `native_connections::Connections` fixes owner-local
names, demanded cross-owner binding addresses, typed native requests and public
namespace routes before body emission. Placement compatibility and emission share
one declaration/import name plan, preserving inference-sensitive source names
and native read-only import writes. Same-owner references stay lexical.
Only private bindings actually crossing owners receive internal exports. If
those names would widen an observable namespace, one exact public projection
wraps that body; it is not a per-source facade family. Static namespaces, dynamic
imports, entry aliases and require-of-ESM use the public route, while lexical
imports use actual body owners. `Artifacts::modules` reports namespace routes;
segment and module members still report their actual binding/evaluation owners.
Body-free private evaluation connectors, including connector-only cycles, are
reduced over the reconstructed typed artifact graph in each entrance's request
order. Cycles containing or reaching a body/native identity retain their
evaluation and shared failure barriers; async closures keep their native requests.
The final phase proof traverses the actual typed static requests after connector
pruning and runtime insertion, including public projections and root routes.
Original native linking
clauses remain for unknown external surfaces and unchanged async request
accounting; opaque CommonJS retains its separate factory/facade activation
contract. These conservative boundaries are not private-address workarounds.
Lexical names are allocated in path/declaration order, avoiding nested bindings,
free references, keywords and helpers. Observable function/class/inferred names
remain separate from those allocations. Namespace,
default-expression/anonymous-declaration and opaque CJS values retain typed graph
identities, not invented lexical binding IDs; cached recipes own default
hoisting/initialization semantics.
Each linked emission prepares graph-wide occupied names and deterministic
default-expression names once, sharing them across placement checks and native
owners. Owners keep only mutable routing/name maps and local name additions;
they do not clone the global exclusions.
Canonical definitions are sorted once by path/binding offset and partitioned by
actual segment ownership. Local naming and private exports reuse each owner's
ordered definitions, including relocated bindings, rather than scanning and
sorting all graph definitions per owner.

## Real browser application smoke tests

`cargo build -p bundler-cli` followed by
`node scripts/bundler-browser.mjs` runs pinned TodoMVC Modern JavaScript and Web
Components sources and native published output through `agent-browser` in real
Chromium. See `fixtures/bundler-apps/README.md` for prerequisites, exact packaging
adaptations, behavior checks, licenses and retained screenshots/diagnostics.
This browser corpus is separate from the Node-only portable semantic fixtures.
Both apps have no lazy imports: their browser contracts require exactly one
loaded JavaScript output, with CSS resources and separately requested CSS entries
outside that count.

The bundler CLI's optional `--css-auto-mount` argument selects native ESM CSS
stubs for browser applications; its existing default is unchanged. Caller-loaded
CSS URL wrappers use CommonJS and cannot execute directly in browsers.
Inferred nested entry names normalize host separators before portable-name
validation; explicit `Entry.output_name` values still require slash-separated
portable paths. Publication validates path components rather than rejecting
Windows separators introduced by native path joins.

### Deferred initializer reconstruction

`link::deferred_esm` explicitly reconstructs one checked synchronous, acyclic,
single-root/single-chunk ESM island. It returns `Some(DeferredEsm)` with ordinary
native artifacts and the generated initializer export name, or `None` when the
island needs native evaluation. `link::esm` and `BuildSession` also select
eligible single-parent dynamic islands automatically, as described below.

Supported retained declarations are named root functions and simple identifier
`var`/`let`/`const` declarations (including split declarators and named exports).
Static internal imports/reexports compose in dependency/source order. Functions
stay at the ESM root and close over the same root bindings; variable bindings
become root `var`s, with assignments/effects inside one lexical guarded initializer.
The native export list remains live, not a returned factory namespace. Merely
loading the artifact does not initialize the island. Success and every failure,
including `throw undefined`, are cached permanently; reentry is rejected.
This deferred reconstruction mode does not preserve TDZ errors.

CommonJS, externals, TLA, cycles, dynamic/require dependencies, observable
namespace uses, default declarations/expressions, classes, destructuring,
unsupported declaration statements and name-sensitive collisions stay native.
Imported writes and writes to const bindings also prevent reconstruction.
Cached source-local declaration alternatives and original-coordinate initializer
slots use the same checked operation renderer as ordinary native emission;
no emitted-source parsing or runtime source loading is involved.

The internal `link::esm_plan_activated` reconstruction seam takes explicit
source-file → physical evaluation-owner selections. Ordinary prepared emission
derives parent selections from retained dependency uses without an enable flag.
Explicit callers must certify that dynamic
namespace results are used only for live reads, call receivers and identity,
not reflection, mutation, escaping to unknown consumers or namespace reexports.
Those unsupported observations must retain their native boundary. Eligible
results use one per-island null-prototype getter literal over root bindings,
not export snapshots or a namespace runtime.

Selected static dependency closures receive separate cached guards. Synchronous
activation calls run inside each dependent guard in source occurrence order;
loading availability files alone does not activate them. Cached dependency
operations replace only the original dynamic-import expression. Local imports
yield before calling the local guard; cross-owner imports load the native
availability chunk and invoke its exported guard before yielding the same live
namespace view. Guards and views are exported only when another owner needs
them. Typed `Code` requests use final physical owners and survive empty-file
pruning and destination renaming. Deferred availability files have artifact
members and async routes, not independently executable source namespace routes.

The seam rejects unsupported declaration/namespace forms, `then` export
surfaces, source TLA/static cycles, require/context/CJS activation, eager/public
demands on deferred sources and incompatible co-location. It falls back to
ordinary native emission rather than changing their evaluation. Deferred sources
with their own dynamic imports currently stay native.

Owned references and checked source edges now retain per-occurrence
`async_deferred` syntax tags. Retention preserves their source-edge indices even
for mixed async/sync uses of one target; checked source provenance covers the
tags on cold and cached inputs. Tags alone do not change ordinary output.

### Opt-in generated async boundaries

`BuildOptions.output.synthesize_async_boundaries` defaults to **false**. Enabling
it permits new suspension and native load failures at eligible references, even
before an async function's first original await. Exact scheduling against outside
work is deliberately not preserved. Guards, expression position, existing awaits,
argument/spread order, plain-call receivers, shorthand keys, binding identity and
surrounding exception handling remain intact.

The supported subset is private top-level function-declaration islands in ESM
sources without runtime dependencies or TLA. Functions can refer to their own
parameters/locals, globals and other eligible functions; self/mutual recursion
within an island stays synchronous. Bodies may have effects or throw: relocating
declaration availability never executes them. Every retained incoming value
reference must be a checked, read-only `async_deferred` occurrence with an existing
reference recipe. Public exports/roots, retained synchronous consumers (including
nested synchronous callbacks), captures of other top-level declarations, imported
bindings, redeclarations, optional invocations and affected PURE-lowering sites
block extraction. CJS, externals, source cycles, escaped namespaces and
`import.meta` are not newly supported.

`chunks::partition_with_options` applies this policy before physical optimization;
`link::esm_with_options` also accepts the same option on an ordinary plan.
BuildSession exposes the derived plan in its product. Source/cache facts stay
unchanged. Low-level callers must repartition to disable an already derived
plan. `ChunkPlan.generated_boundaries` records source-edge/reference
occurrences and actual segment targets, not fabricated import dependencies.
Selected binding demands and declaration-to-source evaluation demands are
replaced before exact masks and the diagnostic manifest are recomputed.
Unrelated source effects keep their original eager scheduling.

Generated cross-owner uses load native segment artifacts and read direct bindings,
without source-module namespace facades or runtime registries. Requests remain
typed through naming/pruning. Ordinary parent-hoisting and library clustering
still run. With a nonzero `min_chunk_bytes`, an undersized function-only island
may also share its unique same-source evaluation parent; a local await remains
observable even when no download is needed. Pure function declarations need no
initializer guard. Existing physical deferred-module merges still use only their
targeted guards. Configuration transitions reuse parse/resolution facts and
invalidate derived output; unchanged enabled builds remain no-ops.

### Automatic single-parent placement

A retained dynamic target with exactly one logical source parent across all its
runtime uses can share that parent's file. Multiple sites in the same parent
count as one parent. Its complete synchronous static dependency island moves
with it only when no outside consumer or public/eager root needs that island.
Existing acyclicity, declaration, naming and deferred-activation checks remain
mandatory. Logical reachability masks do not change; physical artifact members
and typed async routes resolve to the final owner. There is no child artifact or
fetch for a selected island, and loading the parent alone never initializes it.

Selection checks cached AST observations, not just namespace demand. Supported
uses are discarded imports, discarded awaits, direct `(await import()).name`,
`const ns = await import()` with exclusively static property reads, simple
object destructuring, and one-arrow `.then(ns => ...)` consumers with those same
property observations. Live reads and root function/capture identity survive.
Receiver calls additionally require checked, unreassigned function declarations;
islands containing `this` stay native. Escaping promises or namespaces (including returning
`import()` from a public loader), reflection, computed property accesses, writes,
rest/default destructuring, namespace exports and `then` surfaces stay native.
Unsupported TLA, cycles, externals, CJS, nested dynamic dependencies, declaration
forms and observable-name collisions likewise retain native placement.

### Small-island libraries

`BuildOptions.output.min_chunk_bytes` is one approximate uncompressed UTF-8 byte
target; **0 (the default) disables clustering**. Low-level callers can pass the same
`OutputOptions` to `link::esm_with_options`; `link::esm` retains the default.
Changing this option invalidates output, not source extraction or resolution,
and unchanged builds still reuse their immutable generation.

Existing placement-compatible native groups and pure lexical binding islands
use their retained recipe source-piece byte lengths, not trial emission or
compressed-size predictions. Nonempty masks come directly from
`ChunkPlan.node_roots`. Seeds follow source-path/segment order. Individually
undersized candidates rank by Jaccard overlap with the **current union** of the
library's masks; zero overlap is ineligible and ties use the same source order.
After each merge, both the mask union and byte estimate grow. Selection stops
at the target or when no compatible overlapping candidate remains. The last
merge may overshoot, but its estimate stays below twice the target; already-large
islands never participate.

Different overlapping masks can share one native file without changing logical
retention/reachability. Effectful islands reuse the checked, source-ordered,
failure-caching guards above. Synchronous consumers/dependencies receive their
own guards but retain their physical owners unless selected for a merge; this
keeps shared initializer identity and avoids co-activation. Existing parent moves
keep their certified owners. Ordinary native sources remain unwrapped. Pure
binding libraries require a pure native import closure and no new import cycle.
Public/eager activation, TLA, cycles, escaped/reflective namespaces, unsupported
declarations, CJS/externals and resource relocations still prevent deferred
clustering; they retain their native evaluation boundaries. Artifact members,
availability routes and typed requests follow final ownership through pruning,
entry aliases and naming.

### Bounded preparation and output emission

`BuildOptions.execution` and low-level `link::esm_with_execution` accept
`execution::ExecutionOptions::new(NonZeroUsize::new(4).unwrap())`. The default is
one worker; preparation/resolution run on the CPU pool even with one worker.
One build-scoped Rayon pool bounds preparation, resolution, discovery, nested
immutable Ecco expression transfers, fixed-point evaluation and owner emission.
An installed caller pool is reused rather than creating a competing pool.
If that pool exceeds the configured limit, nested Ecco parallel work stays
inline; outstanding module jobs remain bounded by the configured count.
Single-worker fingerprinting and emission stay inline. Emission is additionally capped by
each batch's actual owner count. No environment variable or global pool selects concurrency.

Module preparation uses a replenished, deduplicated canonical-path frontier:
the coordinator integrates each completed job and enqueues separate preparation
and resolution jobs without waiting for sibling jobs. Workers read source
bytes and validate sidecars or extract owned facts using private Oxc arenas and
the session's shared immutable `PreparedDeclarations`. At most the configured
number of jobs, including completed results, are outstanding. Resolution jobs
retain prepared source owners; pending speculative resolution retains immutable
records rather than reparsing. Cycles/shared dependencies are scheduled once.
Precise jobs overtake speculative jobs; in-flight candidates are reused, completed
speculative preparations are bounded to twice the worker count, and unused queued
work is abandoned when the precise frontier settles. Evicted results can be
prepared again on demand. `Counters.speculative_parses` separates unused eager
extraction from emission reparsing.
Resolution, proof checks and route admission run on workers; graph mutation and
Ecco graph construction stay serial. Resolver maps use short guards, with package loading and
borrowed disk-proof validation outside them. Stateful resources
(CSS, JSON and assets) use a canonical serial frontier after the script frontier
drains; CSS/Sass request resolution and context expansion also run off the
coordinator. Scoped jobs drain before resource processing or emission starts,
so stage worker counts never add. The non-Send coordinator stays on its calling
thread; when called from a Rayon worker, result waits yield to that same pool,
including with one worker. Discovery walks stay serial in an installed caller
pool to avoid blocking its only worker.
Source errors are selected by canonical path after script discovery; serial
resource processing stops at its first failure. Cache warnings are sorted.
Failed builds retain the previous successful product but never return it as success.
Invalidations accumulated by failed attempts survive until the next successful build.
Cache provenance checks, atomic writes and final source/inventory rechecks remain
in force, including no-op returns.

Retention, effects/order, placement, global names and cross-owner export
addresses are prepared serially. Checked graphs and owned syntax facts are
already `Sync` and are borrowed without copies or unsafe trait implementations.
ESM evaluation/binding owners and CJS availability/facade owners emit into
private code, routing and name state. CJS factory order within each actual
availability owner remains unchanged. Workers return typed requests, never
publish. Results/errors merge in canonical owner order before serial pruning,
destination/alias selection, rendering and final validation.

Worker parser/rewrite/declaration-fingerprint counts are aggregated back to the caller, including
failed jobs. Debug final validation remains once per executable artifact;
release never validates output. Worker count does not affect fingerprints or
invalidate a successful no-op generation. `Counters.declaration_fingerprints`
reports actual declaration serialization/hashing rather than assuming worker
counts are zero.

The bundler CLI's bounded `--fixture-config FILE` accepts `"workers": 1`
(default), with integer counts from 1 through 64. Use 1, 2, 4 and 6 for forthcoming
cold/warm/no-op/edit benchmarks; hold fixture inputs, cache mode and output
settings constant. It reports `declarationFingerprints` alongside existing
parse/cache counters. `resolutionCacheHits` and `resolutionCacheMisses` count
proven request reuse separately from source `diskHits`; `inventoryDirectoryReads`
counts demand/context enumeration, including final validation.

### Opt-in duration traces

The library emits standard `tracing` spans and never installs a subscriber.
Callers can use a thread-local `tracing::dispatcher::with_default` and an entered
parent span around `BuildSession::build`, `build_and_publish`, or low-level
emission. Both scoped worker stages explicitly propagate that dispatch and
parent. Job spans start after dequeue and end before returning their results;
waiting in a queue is not charged as source preparation or owner emission.
Worker IDs and canonical batch-local owner IDs are numeric; spans never record
source paths/text, entry labels, namespace fragments, options, or cache payloads.
Tracing does not affect cache keys, artifact bytes, counters, or no-op reuse.

The bundler CLI accepts `--trace-file FILE` alongside `--fixture-config FILE`.
It writes a local Chrome/Perfetto-compatible JSON trace, with per-thread duration
spans for declarations/inventory, preparation/read/cache/decode/validation/
extraction, worker resolution, serial planning/placement, owner emission, finalization,
rendering, final validation/rechecks and publication. It flushes after worker
joins and span exits on both success and returned build errors. Stdout remains
the existing build-result JSON (errors remain on stderr).
No administrator privileges, UAC elevation, global subscriber, or OS profiler
are needed: these are application spans written to a normal user-owned file.

Inventory spans separate checked admission, bounded context enumeration and
final consulted-input rechecks. `build.discover_local` measures speculative
multi-root enumeration; it does not become a global validation/config dependency.
Nested durations overlap: use exclusive time when
aggregating them. Small resolver-policy hashing (`build.resolver_policy`),
lockfile parsing (`resolve.lockfile`), per-request static/context resolution, package-format probing,
TypeScript/JSX compilation and serial resource preparation are separate spans.
Request-kind flags are recorded without paths or specifiers.
`resolve.counts` records numeric resolutions/hits/misses/packages for each script
and Sass resolver, including returned build errors; `resolve.recheck` measures
final proof checks. Neither counts discarded requests as successful cache reuse.
No whole-inventory fingerprint span remains.

The destination's parent must exist. Files are opened with `create_new`, never
truncated. Keep traces outside the build root and output tree, or use a
root-level `.bundler-trace-*` filename (excluded from source inventory).
Existing files, published output trees, and parent-traversal paths are rejected.
Use a new filename for every recording.

For a project with an entry importing lodash, create `workers.json` containing
`{"workers":4}` and record from PowerShell:

```powershell
cargo build -p bundler-cli --release
.\target\release\bundler-cli.exe C:\work\lodash-project C:\work\lodash-project\.bundler-out --fixture-config C:\work\workers.json --trace-file C:\work\lodash-4-workers.json entry.mjs
```

Open that local file in a Chrome trace viewer or Perfetto; do not commit or upload
traces. Warm/cold source cache state remains independent of recording.

## JSON modules

Bare-string package `exports` exposes the selected root and arbitrary files
within that package, so nested JSON, CSS and asset requests need no extra
wildcard map. This intentional Node-encapsulation deviation does not affect
explicit export maps or their private subpaths.

`BuildSession` accepts `.json` and `.jsonc` dependencies and entries. `.jsonc`
additionally permits `//` and `/* */` comments and trailing object/array commas,
not other JavaScript/JSON5 syntax. Ordinary `.json` and `package.json` remain
strict JSON. Extensionless resolution probes `.jsonc` after existing extensions.
JSON values are shared
between default ESM imports, dynamic-import defaults and CommonJS `require`;
objects remain mutable and are initialized lazily when required. Inputs are
validated as JSON, then prepared as an opaque CommonJS value initializer for Node
or a native ESM default value for Browser, not loaded or parsed at runtime.
Browser static and dynamic defaults share the same mutable value.
Computed object keys preserve own `__proto__`
properties, duplicate-key semantics, negative zero, large numbers and escaped
surrogates without consulting mutable `JSON.parse`. UTF-8 BOMs are accepted.
Named JSON exports and import attributes remain unsupported; use a default import
or `require`. A JSON entry defaults to `.cjs` on Node and `.mjs` on Browser.

Resource preparation separates original input bytes from generated JavaScript.
Graph identities remain original resource paths, while cached spans and sealed
provenance belong to the generated parser path/text. Warm sidecars reuse those
facts without extraction. Input rechecks, invalidation and publication still
track the original files; emission never reconstructs or reparses a wrapper.

## Asset modules

Imports of non-JavaScript/TypeScript/JSON/JSONC/CSS files copy exact input bytes to
content-addressed `assets/<sha256>.<extension>` paths. Default imports and
`require` return a public URL string. `BuildOptions.output.base_url` defaults to
`"./"`: these URLs are relative to the **build output root**, not the importing
chunk. Resolve relative URLs against your output-root URL; renaming entries does
not change them. `OutputOptions { base_url: "/static/".into(), ..Default::default() }` instead exports
`/static/assets/<hash>.<extension>`; HTTP(S) CDN bases and relative subdirectories
such as `"./public/"` are also supported. Directory bases gain a trailing slash,
and URL paths are percent-escaped without double-encoding existing escapes.
Empty bases, protocol-relative URLs, non-HTTP(S) schemes, credentials, malformed
hosts/percent escapes, queries, fragments, backslashes and control characters
are errors. Public bases never rename owned resource files or executable import
requests. Equal bytes with equal extensions share one resource file.
Asset entries emit a `.cjs` URL-exporting wrapper on Node and a native `.mjs`
default export on Browser. Unreached dynamic
assets are not emitted.

`Artifacts.files` retains executable/metadata strings; `Artifacts.resources`
contains byte-preserving non-executable files. Use `Artifacts::file_bytes()` to
write the complete output or use the publisher. Resources participate in content
hashing, immutable generations, cancellation, ownership validation and stale
cleanup. They never serve as executable module routes.

## Extracted CSS

CSS default imports and `require` export a stylesheet URL using the same base;
side-effect imports also extract the file. `BuildOptions.css.load` selects the
built-in `CssLoad::CallerLoaded` policy by default (`"caller-loaded"` when
serializing `CssOptions`).
Global CSS uses a CJS URL wrapper and `.cjs` entry extension on Node, or a native
ESM default URL export and `.mjs` entry extension on Browser.
`Artifacts.stylesheets` maps retained
input stylesheet roots to owned output-relative `.css` paths, never public URLs
or executable aliases (including custom-suffix CSS Modules). The bundler CLI reports
this map and resource paths. Callers load the relevant root stylesheets in their
application's order; there is **no implicit DOM injection** or automatic loading
when a dynamic JavaScript chunk executes.

Opt in with `CssLoad::AutoMount` (`{"load":"auto-mount"}` in `CssOptions`)
to emit a native ESM stub instead. At module evaluation it synchronously creates
and appends a `<link rel="stylesheet">` to `document.head`; no stylesheet readiness
is awaited or promised. Without `document` it throws
`CSS auto-mount requires document`; a missing/unusable head also fails rather
than silently skipping insertion. Global CSS's native default export is the
resolved absolute stylesheet URL string; requiring it returns the ESM namespace
with that string as `.default`. CSS entries default to `.mjs`.

Auto-mount relative bases resolve from the emitted **generation root**, anchored
by the actual final owning chunk's `import.meta.url`, including nested entry
renames and aliases. Root-relative bases use that URL's origin; HTTP(S) CDN bases
stay absolute. Neither source paths nor the document URL/base participates.
Owned resource paths remain unchanged: a custom base must actually serve those
resources at the configured address. Mounts survive unused imports and inferred
DOM purity. Static dependencies mount in native evaluation order, while dynamic
imports and deferred requires mount only when activated. Native module caching
gives one insertion per module evaluation, including aliases/repeated imports;
there is no DOM-global deduplication. Separate published generations evaluate
independently and default relative URLs remain inside their own generation.
CSS `@import` dependencies are still loaded by CSS, not additional JS mounts.
Runtime regressions use Node and a DOM recorder (plus native VM modules for
HTTP-origin URLs); they do not test browser CSS application or network readiness.
Publication regressions also exercise shared CSS Modules, renamed aliases and
nested static/dynamic/CJS activation across two immutable generations, checking
mapping values and local stylesheet/asset references against actual emitted URLs.

`cssparser` tokenizes CSS requests, including escaped URLs, quoted/unquoted
`url()`, `@import` and `image-set` strings. Local imports remain separate CSS
files: source order, media queries, supports conditions, layers, charset
declarations and import cycles are not flattened. Local asset URLs are copied
and rewritten relative to the emitted stylesheet for relative bases (the base
is not applied twice), or through the configured root-relative/HTTP(S) base,
retaining queries/fragments;
percent-encoded filenames resolve to original files. Remote, data, fragment and
host-root-relative URLs remain unchanged. Namespace/document URLs remain
identifiers rather than file dependencies. Local files must be inside the same
bounded inventory as JavaScript sources.

Stylesheet names include the complete reachable resource/composition closure and
resolved composition identities, so edits to imported/composed CSS or referenced
binary bytes invalidate parent stylesheet URLs, even through CSS import cycles.
All these input bytes are rechecked before accepting a build.
These resource identities independently invalidate output reuse, including
CSS-only edits that leave caller-loaded module mappings unchanged.
CSS discovery and composition evaluation jobs settle before scoped names,
closure URLs and wrapper relocations are finalized, so later checked aliases
cannot leave wrappers pointing at nonexistent artifacts.
CSS is tokenized on each build; generated JavaScript wrappers reuse the normal
memory/FlatBuffers cache. There is no CSS concatenation or minification.

#### Sass inputs

`.scss` and indented `.sass` files compile natively with pinned **Sasso 0.21.0**
before CSS Modules, URL extraction and loading-wrapper generation. JavaScript can
import them directly; no Sass executable or external preprocessing is required.
`@use`, `@forward` and Sass `@import` load through a bounded importer: relative
partials, directory indexes and import-only `.import.scss`/`.import.sass` files
follow Sass precedence and ambiguous matches fail. Exact resolver aliases take
precedence. Bare package requests (also `~package/path`) use the normal module
resolver with `sass`/`style` export conditions and stylesheet extensions; package
exports remain authoritative. No implicit load paths, Node polyfills or external
Sass modules are supplied.

Original transitive Sass bytes are tracked separately from compiled CSS. Their
edits, additions/deletions and resolution changes invalidate stylesheet identities,
including when the generated JavaScript mapping stays unchanged. All original
inputs are rechecked before success; compiler diagnostics join build warnings and
errors retain source locations. Plain CSS `@import` remains a separate resource,
not a Sass module. Relative emitted CSS URLs resolve against the entry stylesheet,
without rebasing them to the originating Sass partial's directory.

With `output.source_maps: true`, both plain CSS and compiled Sass resources have
external sibling `.css.map` artifacts and CSS `/*# sourceMappingURL=... */`
footers with relative, URL-escaped map paths. Each map's `file` names the final
stylesheet. Sasso's native compiler mappings retain checked original source
file URLs and `sourcesContent`, including imported/forwarded partials supplying
variable values and mixins. CSS Modules selector/keyframe/composition edits and
CSS URL/import rewrites compose into that map against the final emitted bytes;
compiler-supplied mapping granularity is preserved, not invented. Synthetic
JavaScript stylesheet loaders are not attributed to Sass/CSS source bytes.
Maps are published as resources alongside their stylesheets.

Disabled maps perform no Sass compiler mapping work and emit neither stylesheet
maps nor added map footers. Map settings and mapping-only dependency edits
participate in immutable stylesheet identities, so routes, public hrefs and CSS
imports consistently follow the new artifact. Original JavaScript stays warm
when toggling maps; a changed synthesized href loader can require reparsing
that generated module. Unchanged generated module text reuses its normal cache.

Sasso is a v0.x implementation with documented upstream conformance gaps, not an
unconditional Dart Sass parity guarantee. Compilation failures are returned, never
replaced with raw Sass or empty CSS.

`BuildOptions.css.css_modules` (`CssModules`) accepts `false` (default/`Disabled`),
`true` (`Enabled`, selecting `.module.css`, `.module.scss` and `.module.sass`),
`"everything"` (`Everything`, all CSS/Sass inputs), or
`"ext:.any.custom.extension"` (`Extension` with the dotted suffix).
Suffixes consist of nonempty dot-separated ASCII letters/digits/underscore/hyphen
components; empty suffixes, separators, whitespace and URL syntax are rejected
before graph loading, including programmatically constructed invalid options.
Custom suffixes identify CSS before generic asset classification, for entries,
static/dynamic imports, require/context candidates and transitive CSS imports.
Suffixes such as `.module.cjs`, `.module.ts` and `.module.json` still use the
sealed generated JavaScript parser mode, not the original extension's mode.
`"everything"` does not reclassify JavaScript, JSON or binary inputs as CSS.
Each imported stylesheet uses its own selection rule, not its parent's flag.
`BuildOptions.css.css_modules_exclude` is a default-empty list of dotted filename
suffixes using the same grammar. Exclusions override module selection, including
`Everything` and custom suffixes, without changing a selected file's stylesheet
kind. For example, `[".global.css", ".global.scss", ".global.sass"]` keeps those
selectors global while other styles remain scoped. Matching is case-sensitive
and uses the filename only. Excluded styles retain the global stylesheet
URL-default export contract, not a CSS Modules mapping object. The same policy
applies to entries, transitive CSS imports and composition targets; composing
from an excluded stylesheet is an error.

Selected modules scope classes and IDs to `_` plus ten base64url SHA-256 digest
characters. The versioned `css-local-v1` hash uses the normalized project-relative
input path and decoded selector name, never checkout paths, CSS contents or
discovery order. An equal class/ID name in one file shares a value; distinct file
paths have distinct identities. Detected scoped-name collisions fail explicitly.

Mappings are separate pure native ESM named bindings, with a default object
whose own properties contain the same mapping values using the same normalized
names. The default object is shared across static/dynamic imports and supported
require namespaces; requiring a module still returns its namespace, not just
the mapping object.
It is empty when there are no exported locals; explicit globals and animation
names do not become mapping properties. A local whose normalized name is
`default` is available as `styles.default` on the default-imported object, not
as a separate named export conflicting with that object.
JS export names always use lower camelCase, recognizing hyphen/underscore
separators, lower-to-upper transitions and acronym word boundaries
(`my-class-name` → `myClassName`, `XMLHttp` → `xmlHttp`).
Other punctuation is preserved rather than silently dropped. Keywords and
nonidentifier names use safe private bindings and string export aliases. Colliding
export names report both decoded originals and original byte spans; empty export
names are errors. For example `.my-class-name, #my-class-name` exports
`myClassName` with the same scoped string for both selectors.
Named/namespace imports, dynamic imports and CJS require-of-ESM expose those
bindings; a deferred import/require still defers auto-mount activation.
When the default object is unused, it and unused named bindings can be pruned
independently; demanding the complete object retains its mapping values.
This never deletes stylesheet rules or the extracted CSS resource.
Both loaders use identical mappings and CSS bytes;
auto-mount retains its DOM effects but does not add a URL default to modules.
Selection is independent of `css.load`. Disabled selection preserves ordinary
global CSS, including `.module.css` filenames.
The separately pinned upstream `oxc-css-parser` revision
`6e1604ddd0ad0d08279aac4a2910ff8ca585f7c6` parses module selectors, decoded names,
scope markers and composition requests into owned facts with checked original
UTF-8 byte spans. The owned boundary normalizes the pinned decoder's NUL values
to CSS's U+FFFD replacement character for names and composition file requests.
It requires Rust **1.95 or newer**; the repository's
`nightly-2026-05-01` satisfies this. Its re-exported allocator is separately locked
at 0.137.0, independently of the JavaScript Oxc pin.
Recoverable parse errors, raw selectors/unknown pseudo arguments and unsupported
selector-bearing at-rule preludes fail closed with path/span diagnostics.
Classes/IDs default to local. Bare `:local`/`:global` switches the rest of its
complex selector, resetting at each list comma; functional forms scope only their
selector arguments. Both forms are removed from emitted CSS. Lists, combinators,
nesting, selector-bearing pseudos (`:is`, `:not`, `:has`, nth matchers and slotted
selectors), `@scope`, `@nest` and `@supports selector(...)` preserve their selector
boundaries. Functional lists expand into alternatives, not `:is()` wrappers that
would change specificity; relative `:has()` alternatives each retain their leading
combinator. Expansions exceeding 4096 alternatives per complex
selector and ambiguous type-selector joins such as `.a:global(div)` fail closed.
Expanding lists into a singleton-only context (for example `@supports selector()`
or `::slotted()`) also fails rather than emitting an invalid selector.
The pinned parser leaves functional `:local(...)` arguments raw; the owned AST
boundary reparses only these scope arguments as complete rules, rejects recovery
or trailing syntax, and translates their spans back to original UTF-8 bytes.
Only local names become exports; globals are neither renamed nor exported.
`url(#id)` and quoted/escaped equivalents use normal URL whitespace normalization
before rewriting only a known local **ID**,
not a class-only name. Unknown/global fragments, external URL fragments,
attributes, strings, comments and unrelated identifiers remain untouched.
Every transitive `@import` uses its own suffix/selection rule, never inheriting
module mode from its importer; imports and their conditions remain separate.
Module `composes` declarations resolve stylesheet-local forward references and
explicit `from global` names, then disappear from emitted CSS. Each mapping keeps
its own scoped name first, followed by composition tokens in declaration order
with deterministic deduplication. Targets must be single local class selectors
(including explicit `:local` forms), not selector lists, compound/complex
selectors, IDs or nested rules. Unknown local names, illegal placement and cycles
fail with source paths/byte spans, even for unused exports. Cross-file
`composes: base-name from "./other.module.css"` uses the same bounded import
resolver/inventory as JavaScript, including package exports and CSS string
escapes. Names refer to decoded original local selectors, not their camelCase JS
aliases. Targets must be selected CSS modules exporting that local name,
classified as stylesheets by their `.css` or configured custom suffix. Unlike
CSS `@import`, `composes` cannot treat arbitrary filenames as stylesheets,
even under `"everything"`;
missing/global-only names, non-module targets, externals, private package paths,
unlinked out-of-root files and excluded files are errors.
Composition closure discovery precedes loader generation. Mappings resolve
dependency-first; composition file cycles fail even for otherwise unrelated
names, while ordinary CSS `@import` cycles remain supported. Generated native
static evaluation imports mount composed dependencies before their consumer,
once per native evaluation, without activating a lazy consumer early. Styles
discovered only through CSS `@import` (including their composition closures)
remain render-only: conditions never become unconditional JS mounts.
Composed mapping changes invalidate consumer source records and reverse users
even when consumer input bytes do not change. Resource-only edits invalidate
closure/output identities without reparsing unchanged caller-loaded mappings.
Module-local identifier `@keyframes` names (including nested/vendor-prefixed rules)
and matching `animation-name`/`animation` references are scoped through Oxc
declaration/value ASTs. Animation identities use a separate `css-animation-v1`
domain with the same project-relative path/symbol hashing; they add no JS exports.
Explicit `local(name)`/`global(name)` and `:local(name)`/`:global(name)` escapes
select reference/definition scope and disappear from output. Each escape requires
one identifier; nested, ambiguous or unsupported module-only values fail with
path/span diagnostics. Unmarked references rewrite only known local keyframes.
Shorthand lists disambiguate timing, iteration, direction, fill and play-state
slots before interpreting an identifier as a name; `auto` is an ordinary name
candidate. Explicit global keyword names emit as strings to retain their name
semantics in both shorthand and `animation-name`. Removing scope wrappers keeps
adjacent tokens lexically separate. Timing functions, strings (including quoted
keyframe names), custom properties and opaque runtime values
remain intact; an opaque shorthand group is not guessed, so use an explicit
scope when its animation name must be local. ICSS `:import`/`:export` and `@value`
remain unsupported, not partially transformed. Ordinary global-CSS animation
and `composes` declarations remain untouched.
Extraction is stylesheet-wide: pruning JS mappings never removes CSS rules or
changes cascade order.
`cssparser` still owns URL tokenization; URL and selector
edits share one overlap-checked original-byte pipeline. Resource names include
module transformation identity throughout the CSS import closure so global and
scoped bytes cannot share an artifact name.
That identity includes the selector/animation implementation and CSS parser
revision, not the loading strategy or its DOM stub. Equivalent suffix selections
therefore share resource names; changing loaders never renames stylesheet bytes.

Resource discovery/rendering and loading-stub generation are separate: stubs
receive the stylesheet identity, public URL and independent name mappings,
not CSS syntax. Prepared wrapper text and explicit parser identity travel
together into cache loading. Emission checks sealed parser provenance and
plain-JavaScript support without parsing or assuming CSS always means CJS.
Auto-mount additionally carries exact generated construction-operation spans
into typed assembly URL parts, resolved after pruning, ownership and final
naming; no emitted-string placeholder scanning is used.
Base-only edits regenerate asset/CSS URL wrappers before memory-cache comparison;
generated text keys parse reuse, while the normalized base also keys output
reuse. Unchanged JavaScript stays warm, and owned resource paths stay unchanged.
Selection changes recheck the complete CSS/composition closure before comparing
prepared text. Every input compares its newly prepared text, including custom
stylesheet suffixes deselected back to JavaScript; stale mappings cannot bypass
source validation. Declaration-only CSS edits can reuse generated mappings from
FlatBuffers, but never skip original-input checks or stylesheet output invalidation.
No cache stores linked graph handles or solved effects; no extractor/schema bump
is needed for these resource-layer identities.

## Native ESM chunks

Public `link::esm`, isolated emission and `BuildSession` share parse-free source
verification. Graph inputs must preserve extraction/cache provenance through
`OwnedSourceFileInput::from_parsed`; analysis-only `from_ranges` inputs cannot be
emitted. Exact source hashes, effective parser modes and owned records must
match; JavaScript support and graph/policy revision checks remain mandatory.

Low-level `link::esm` emits **one `native.mjs`** for a single namespace
entry/evaluation root whose retained static
dependency tree is synchronous, acyclic and ESM-only. Named/default binding
imports and reexports, including pure star barrels, become direct canonical
lexical references. Default expressions have private snapshot bindings, while
`export {binding as default}` remains live. Anonymous default functions are
created before source evaluation (safe only in this acyclic single-file slice);
anonymous default classes/expressions initialize at their original evaluation
point. Native object-property named evaluation preserves their `default` name
without a name setter, including class static initialization. Named default
declarations retain their own binding and function-hoisting/class-TDZ behavior.
Public names are explicit output export aliases; private
exports and import declarations disappear. Declarations, all retained var
contributions and statements stay in source order within dependency-first
static-import DFS order, never path-sorted body order. Cached segment recipes
compose PURE lowering, shorthand, namespace reads, default initialization and
binding/dependency substitutions in original UTF-8 coordinates. Replacing
operations explicitly own their nested source slots; no retained-source AST or
name/specifier reconnection is needed.
Multi-declarator variable statements are independently retained declarator
segments. Both native emission paths and isolated emission materialize their
cached `MultiSpan` pieces, reusing the original declaration/export prefix and
adding a terminator. Binding/effect ownership stays disjoint; shared source
prefixes do not create graph edges or merge declarator lifetimes.
PURE guarded alternatives use cached callee steps, source slots and await-guard
restrictions without parsing callee fragments. Annotation policy and result
demand still select whether an invocation is retained or lowered.
Shorthand keys, free globals, nested shadows, hoisting, strict-module `this`,
TDZ, const and live mutation keep native semantics. Debug builds parser/semantic
validate each final executable artifact once after naming and aliases; release
emission never parses source fragments or output. Checked source provenance,
identifier legality (including strict reserved names), edit ranges and artifact
targets remain enforced in both profiles. Cached in-segment source-hint operations
remove stale hints without output comment scans; out-of-segment hint cleanup is
not supported. Output has no cells, registrations or per-module facade family.

Public `BuildOptions.entries` keys are logical IDs. `Entry.output_name` is an
optional relative output name/path supplied by the build config. When omitted,
the output preserves the canonical source path relative to the project root:
input `"./entry.mjs"` emits `entry.mjs`, regardless of its entry ID.
Nested default outputs use `/` separators on every platform, including Windows.
Explicit output names must also use `/`; native backslashes, drive prefixes and
UNC paths are rejected rather than normalized.
`output_name: Some("appstart".into())` emits `appstart.mjs` for that input;
`Some("app/start.mjs".into())` preserves the nested output path. Missing extensions inherit supported
input `.mjs`/`.cjs`/`.js` extensions, otherwise the emitted ESM/CJS format.
Explicit extensions must match that format (`.js` supports either format through
a generated `package.json` in its output directory; conflicting `.js` formats
in one directory are rejected).
Absolute, traversing, reserved, nonportable, file/directory-colliding and
generated-artifact-colliding output paths are errors.

`BuildSession` renames the actual canonical entry chunks and all their generated
relative requests and artifact metadata, including on cached builds and
publication. A single `.mjs` entry remains one file, not a wrapper plus
`native.mjs`. Static and dynamic imports still target that same canonical native
namespace. Logical aliases with the same output path share that canonical file.
Destinations and aliases are selected on typed artifact identities before source
rendering. Static imports, reexports, dynamic imports and checked CJS require
sites carry typed targets; arbitrary source strings are never discovered or
rewritten as requests. ESM requests use URL encoding; require uses filesystem paths.
When several explicitly named entries request distinct paths for one namespace, the first key in
sorted order names its canonical chunk; the other requested files are native
reexport boundaries (forwarding default and live exports), not additional
executions of its body. Their namespace objects are distinct, as with ordinary
ESM reexports; internal dynamic imports retain canonical namespace identity.
CJS aliases forward the canonical wrapper's current exports, including
reassignment and accessors. Published generation directories do not change these
relative entry names. Low-level `RootId` labels retain their existing contracts.

Checked dot-property reads of namespace imports (including namespace reexport
aliases) resolve directly to canonical live values when the whole graph can
concatenate. The broader ESM path emits real body-bearing
`native/<path-hash>.mjs` chunks, with ordinary static imports/reexports between
them. It supports multiple entries, evaluation-only entries, escaped namespaces,
computed reads, descriptors, receivers and read-only writes. Namespace values
are actual native module namespaces, never getter objects or setter facades.

There is no greedy whole-module or complete-SCC merging pass. Unobserved pure barrels
redirect to their dependency only when their complete export-name/canonical-
identity maps match, including `default`; subset surfaces must not widen later
star exports or introduce ambiguity. Each physical owner has an explicit,
complete, exactly-once `SegmentKey` schedule, checked against mandatory constraints.
Emission materializes cached full MultiSpan recipes in that schedule, including
disconnected retained components; module DFS never discovers executable bodies.
Shared dependencies remain once-only binding owners.
Public entry surfaces and namespace targets keep precise native
surfaces; internal imports/exports disappear inside a physical owner. No generated
private names leak into namespace keys.

Physical owners are independently assigned to retained segments, not input
modules or root-set partitions. Pure lexical components move to a unique consumer
or independent shared binding chunks; components from unrelated sources can
co-locate. Eager local initializer reads and writes keep their lexical components
together, preserving snapshots and mutable closures without cells or setters.
Source-private visibility must not make a deferred reader and its declaration
an indivisible component. A pinned initializer and a shared deferred reader may
have distinct owners even when the source namespace is observable.
Source-phase barriers pin non-hoisted evaluations, even when Ecco or PURE calls
classify a site as pure. Proven initialized immutable local reads can discharge
read-only fences; their mandatory ordering still travels with the component.
Hoisted body-free functions can leave effectful sources without moving their effects.
Pure initializations before a source's first abrupt phase can also share binding
owners: the native source imports those jobs after its original static requests
and before its body. Mandatory prefix-to-phase edges are preserved by those
explicit native dependencies, not discarded; later initializations stay fenced.
SCCs, async closures, forward references and opaque inputs do not use this proof.
Physical import contraction is checked against entrance-specific native body
events and lexical initialization visibility, including transitive activation and
existing native cycles. Every body event must observe the same preceding lexical
initializations, so throwing dependencies cannot eagerly expose or delay an
export through an escaped namespace. Independent pure initializers may commute
between these events. Unsafe candidates are anchored, not repaired by breaking
mandatory edges. Sparse owner references are indexed once per ownership revision;
candidates sharing a job reuse its scalar activation summary without storing
transitive symbol sets.
Static closures below top-level-await jobs keep their original native request
graph: a scalar completion projection cannot prove host async pending-dependency
accounting or sibling interleaving. Hazardous mutable/deferred sources otherwise
remain conservative; this planner does not infer named-call execution or purity.
Name-sensitive components split into separate chunks when co-location would
change names or capture references. Consumer rewrites and artifact members follow
actual segment owners, including aliases and default-expression addresses.
Placement and candidate-name compatibility consume owned source-local recipes,
without reparsing or trial emission. Connector classification includes only
retained segments; dead declarations do not prevent connector grouping. Namespace,
cycle, initialization and evaluation-order barriers remain graph-derived.

### Semantic obligations before physical routes

The durable pipeline is to lower original static imports/references into typed
obligations for binding availability, mandatory execution order/effects,
activation/deferred/top-level-await timing, exact namespace/public surfaces and
external native validation. Solve physical segment ownership from those
obligations and consumer demand, independently of source visibility or existing
export addresses. Then reconstruct same-owner local references and cross-owner
internal native imports/exports, and project each observable namespace exactly.
Original import obligations survive; their literal source-module topology need
not. A deferred binding dependency is not eager execution, and scheduling edges
must never supply retention.

Internal chunks may export source-private bindings for other owners. Those
internal routes must not widen public namespace keys; native public projection
wrappers are appropriate only where an actual observable surface needs one, not
as a mandatory facade family per source. Real execution/binding compatibility
constraints may still require co-location. No cells, registration, interpreter,
source loading or runtime compilation is permitted.

This supersedes the earlier rule that observable namespace seams force private
declarations and deferred readers together. Private cut-edge addresses and public
projections allow a pinned initializer and shared reader to keep distinct owners
without widening the observable namespace.

Namespaces and CJS-consuming modules no longer impose whole-module placement
barriers. Namespace demand retains checked visible exports and ordinary effects,
not unrelated private declarations. Empty private evaluation chunks are removed,
their evaluation imports are spliced in original order, and no-op metadata owners
are remapped to surviving consumers. Public, escaped and dynamic namespace
boundaries remain native modules with exact keys and identity. Reduction uses SCCs
of the reconstructed typed artifact graph, not source import cycles. Body-free
private evaluation SCCs disappear, expanding outgoing obligations in each
entrance's source order; independent entries gain no global order. A component
containing a retained job keeps its connectors so a revisit cannot bypass that
still-evaluating job's dependency order. Native binding/namespace requests and
external linking clauses also keep their identities.
The physical plan carries source fragments and typed ordered requests, not an
emitted AST. Cached retained-content flags select empty private jobs before final
rendering; binding requests cannot target a deleted job. Ownership projections
are updated with the spliced evaluation edges.

Initialization constraints remain real: effectful statements, eager mutable reads
and non-hoisted initialization across cyclic/async phases stay at their native
evaluation phase; hoisted functions can relocate when their new static-import
closure cannot activate a later native initializer early. Native anonymous
defaults preserve instantiation semantics inside their own evaluation chunks.
Imported writes still address read-only native aliases. Source evaluation groups
preserve effect/await order; they are not physical ownership units for independent
declarations. This is not a claim of globally optimal chunk sizes. Upstream
parity remains deferred.

Literal dynamic imports and source TLA/for-await now use this same planner.
Dynamic source occurrences remain distinct logical roots, but every import of
one target uses its canonical actual chunk path and native namespace. Only the
specifier is rewritten: Node owns promises, microtasks, exported-`then`
assimilation, shared rejections and cached identity. Dynamic edges never become
static imports or permission to invoke a nested body. Effectful deferred targets
remain inactive until requested.

Explicit `BuildOptions.externals` remain native requests, including builtins.
Static/default/named/namespace imports, named/namespace/star reexports, dynamic
imports and CJS requires retain the declared specifier verbatim; no external
source is loaded or copied during bundling. The deployment supplies those
requests relative to each emitted job. Host resolution, live bindings, namespace
identity, require caching, load errors and async-module restrictions stay native.
Unknown external star surfaces are not enumerated or guessed; ambiguous routes
involving them defer to native linking while retaining every candidate route.
Static import/reexport declarations remain even for unused bindings, preserving
native missing-export checks. Mixed CJS stars retain their demanded proxy names
as star contributions, never explicit exports overriding external ambiguity.
Native source-module seams are conservatively kept across a graph containing
externals, since unknown export/evaluation behavior cannot justify concatenation
or lexical relocation. External execution also disables inferred Ecco pruning
because it can mutate otherwise-pure standard globals. Source PURE policy remains
separate. These restrictions do not introduce a second emission backend.
External nodes remain in the diagnostic manifest without invented local files;
`async_facades` and artifact ownership maps contain local targets only.

Relative template/concatenation imports and requires capture a closure from the
bounded project inventory, not runtime filesystem searches. A leading literal
`./` or `../` is required; runtime gaps match any path depth between literal
pieces. Runtime keys are exact relative spellings; imports capture URL-encoded
keys as well as unescaped spaces/Unicode, without confusing URL keys with
filesystem paths.
Requires also capture configured extensionless and `index` aliases, not arbitrary
package-directory aliases. Inventory additions/removals invalidate resolution on
warm builds. Each context retains its source occurrence; async roots additionally
identify each candidate without merging their public namespaces.
Generated switches evaluate the original argument once at its original site,
composing cached nested binding/dependency edits and preserving source awaits.
Only the selected native import or cached CJS initializer executes. Unmatched
requires throw synchronously; unmatched imports reject through native specifier
coercion. An empty closure is not a successful empty namespace. Context dispatch
shares the captured intrinsic shim with CJS even in ESM-only builds, so user
bindings or later mutations of `Error` cannot intercept generated lookup errors.

Full dynamic namespace observations retain their complete public surfaces, without
private exports; checked selected observations use occurrence projections.
Source TLA status propagates over static runtime dependencies, including
SCCs. These async modules stay body-bearing native jobs rather than concatenating
across source wait boundaries. Source await/for-await syntax is untouched; static
siblings interleave natively, never through scalar effect-order dispatch,
generators or universal async wrappers. Compatible synchronous subgraphs still
use the existing grouping/lexical-rewrite path.

Opaque CJS factories now participate through the existing typed body/proxy/
namespace nodes. Pure factory chunks group by availability roots, never execute
their factories on load, and contain one lexical cached initializer per input.
Native interop jobs group adjacent sorted facade parts only when their consumer
evaluation demands agree; require-only availability cannot activate these jobs.
ESM sources touching CJS retain native body seams rather than concatenating
across the format boundary. Other ESM groups/jobs are unchanged.

Low-level roots support selected public/private binding segments, binding-only
owners, multiple evaluation/binding owners, empty modules and entry-name aliases.
Single-owner ESM binding roots expose the retained public surface, not private
bindings or a demanded complete namespace unless requested. Evaluation-only ESM
roots and all multi-owner roots use export-free native public entry wrappers.
Their imports follow evaluation-root order, then first-occurrence binding owners;
normal native imports preserve dependency execution, once-only evaluation and
throw/async behavior. Single-owner CJS roots keep executable CJS entry wrappers
and their full default result. Aliases of one public namespace share its
canonical path. Require-of-async diagnostics retain original source spans.

`members` points to actual body owners. A logical `chunk_artifacts` projection
can contain several physical paths and overlap another projection when physical
groups merge; the manifest/reference reachability oracle is unchanged.
`modules` contains only independently executable source namespace surfaces and
public CJS entry wrappers, not fake paths for merged private modules or internal
CJS factories. `async_facades` maps reached source sites to their canonical
target chunks (CJS targets use the canonical runtime-key bridge job);
`async_modules` reports source paths with transitive static async status.
The 67-source star-chain benchmark
still asserts one ordinary public `main.mjs`, without runtime/helpers.

## Retention

Source extraction records owned Ecco functions and at most one lazy root per
segment in the typed metadata FlatBuffer, using the existing AST/semantic parse. After
registering the complete graph, retention solves those precursors together,
resolving imports and barrels through the checked `SourceGraph` export identities.
The same connected effects drive public builds and low-level retention/chunks.
CommonJS stays opaque. Native binding/TDZ barriers and add-only forced effects
remain distinct from Ecco's expression effects. Structural force rules targeting
deferred bodies, opaque CommonJS/external execution, captured lexical initialization/TDZ barriers, or disabling PURE
annotations conservatively disable inferred pruning; cached precursors and
source-level PURE lowering remain unchanged. Ecco's existing unknown-execution
widening can also prevent pruning otherwise pure calls.

`BuildOptions.declarations` enables JavaScript and the requested resolver target's
host preset (Node by default, Browser for `TargetEnv::Browser`) and accepts parsed
user declaration sets. User bindings override presets;
duplicates between user sets and build-wide local contracts are errors. Disable
language or selected host contracts with `javascript`, `node`, or `browser`.
Switching targets also invalidates prepared declarations; explicit user read/call
contracts and language switches survive that transition. The effective parsed declaration
identity participates in disk metadata, memory reuse and build parse fingerprints,
so declaration changes trigger fresh extraction and whole-graph analysis.
Only potentially referenced declaration families enter each file's precursors;
cache identity still hashes the complete effective declaration set.
Public builds also default `module`, `exports`, and `require` global-symbol reads
to effectful, independently of the presets. Explicit user read contracts override
these defaults. The contracts participate in the same validated prepared identity
and propagate through inferred functions and imported helpers; they introduce no
runtime bindings. Low-level graphs with undeclared deferred probe reads
conservatively disable inferred pruning instead.

Library-only retention, exact chunk planning and JavaScript ESM/CJS emission over
the checked owned `SourceGraph`. This is not an unused-report adapter.

`retain(graph, evaluation_roots, binding_roots)` keeps module evaluation demand
separate from binding demand. Every runtime static import/re-export demands the
target's evaluation body, even if its bindings are unused. Type-only sites do
not. Effectful statements keep lexical dependencies; function/default-argument/
instance-field bodies do not become top-level effects.

`RetentionPlan` exposes deterministic retained segments/reasons and additional
consumer → dependency edges: static evaluation, module effects and effect order.
`ModuleEffect` edges run from a shared module-evaluation hub to its local effects.
Live consumers reach that hub through `BindingEvaluation`; `StaticEvaluation`
routes retain transitively imported effects, including through pure barrels,
without copying the effect closure onto every segment.
`EffectOrder` edges instead constrain scheduling only: preceding statements and
runtime static imports are projected to actual effects. Hoisted import/re-export
declarations precede executable statements regardless of textual position.
Synchronous sibling static subtrees complete in source order, skipping already
entered modules. Source TLA status propagates through static imports/reexports;
async modules do not contribute sibling completion-chain edges, since later
siblings can execute before their completion. Local source-order and static
dependency constraints remain. This scalar DAG is only a partial order, **not a
full async scheduler**; omitted async sibling constraints belong to native
module jobs. It neither converts ordinary source awaits nor
introduces an unsupported-input gate.
Compact connector/effect chains avoid all-pairs statement constraints. Iterative
connector SCC condensation caches first-effect projections, including empty
ones, once per component; single-successor chains share projection sets. Shared
pure barrels are not re-traversed for each effectful consumer, and projection
stops at effects rather than materializing full transitive effect closures.
Reachability indexes outgoing retention edges once rather than rescanning the
whole graph per node. Order edges do not retain
bindings or add chunk availability/manifest edges. Pure bindings retain their
ordinary dependency graph without being pinned to source positions.
`effect_order` lists available effects dependency-first, breaking order-only SCCs
by canonical source path/span. `warnings` identifies cycle members in the chosen
order; `BuildResult.warnings` includes these on changed **and no-op** builds.
Binding/link/TDZ errors are not exempted. Async imports and deferred requires do
not gain eager order constraints; the list is not permission to activate them.
The emitter uses native module jobs, not this scalar linker-planning order.
`RetentionPlan::execution` adds scheduling-only inputs under the same revision/
policy validity check. Its sparse `constraints` distinguish mandatory local
initialization, original pre-initialization (including TDZ failure), read/write
ordering, and `AbruptCompletion` from breakable `EffectOrder`. Direct `accesses` link
existing reference indices to real declaration segments through canonical export resolution, including
barrels. Deferred accesses remain availability relationships, not eager edges.
Cross-source reads deliberately do not invent declaration-before-read edges: cyclic
evaluation depends on the entrance. `barriers` locate conservative native source-phase
fences for unproven calls, implicit unknown execution, abrupt completion, deferred
captures, cross-source accesses, suspension and opaque inputs. Until a scheduler
discharges a fence, it covers the owning source's
phase, including other mutable reads/initializers. Neither inferred purity nor PURE
hints discharge fences. None of this data participates in root/manifest traversal.
Implicit-execution and abrupt-completion evidence comes from eager syntax-effect
sites in the owned execution context, including tagged templates, getters,
coercions, iteration, potentially throwing reads and explicit throws. Every
site resolves its recorded owning operation context, not its operand's context:
callable class heritage and computed keys still evaluate eagerly, while class
definitions inside deferred callables/instance initializers stay deferred. Each
`AbruptCompletion` phase has mandatory predecessor/successor constraints against
other retained local evaluations/initializations, preserving the complete binding
state when evaluation fails. Adjacent fences compose transitively with linear-size
constraints; peer initializers between fences stay unordered.
Hoisted function declarations and static connectors
are not evaluation phases; these constraints neither repair TDZ nor impose an
order across independent sources. Simple literal initializers remain movable
when no such fence intervenes.
Native placement consumes these relationships directly. Mandatory lexical components
stay together; checked native prefix dependencies and per-owner schedules preserve
their partial order. Only effect-order analysis
may break its own order-only cycles. The four `native-flatten-*` contracts exercise
absorption, outside-return activation, canonical barrels and both SCC entrances.
A bounded single-entrance fast path concatenates only a complete acyclic synchronous
closure with no outside/native namespace boundaries. It preserves original source
job completion order and checks every mandatory constraint against its explicit
segment schedule. Other roots use independent native jobs and binding owners.
Original structural edge indices retain exact occurrence/timing metadata.
Required bodies are **available**, not scheduled eagerly: deferred requires
keep their original call sites/timing. Async boundary edge indices are recorded
but not crossed by retention; `chunks::partition` assigns separate roots. Namespace
demand retains all visible export routes, omitting ambiguous star names and
unrelated private declarations. Opaque CJS bodies remain indivisible.
External edges remain explicit unlinked sites for later output policy.

Construct inputs from `ParsedModule::facts` or decoded cache facts/ranges through
`OwnedSourceFileInput::from_ranges`. Legacy name/usage maps are rejected for
reached modules. Link diagnostics are errors, never success-shaped omissions.
Plans carry the graph revision; recompute after source/resolution changes.
Source order constraints are not a global runtime schedule (especially cycles).
Binding-demand edges also reach their own module-evaluation node, so later root
walks cannot omit effects merely because their initial root is a binding.

Run `cargo test -p bundler`.

Unified source and public-pipeline conformance:
`cargo build -p bundler-cli && node scripts/bundler-fixtures.mjs`.
Harness regressions:
`node --test scripts/bundler-fixtures.test.mjs scripts/bundler-fixtures-build.test.mjs`.
The runner validates the entire corpus before execution, skips builds for bad
inputs, and collects input/build/required-backend/output failures without stopping
later cases. It reports native and legacy passes separately; unexpected failures
make the CLI exit nonzero after reporting all cases. The same command retains
the same-runtime identity and source-range diagnostic checks.
The `bundler-cli` command can also build a project directly:
`cargo run -p bundler-cli -- PROJECT OUTPUT ENTRY...`; it prints
generation-specific `entries`, the selected `backend`, physical `layout` and
actual build counters as JSON. `link::Artifacts::backend` is the authoritative
`link::Backend::Native` value assigned by the emitter, never inferred from
filenames. It serializes as `"native"`; `Backend::layout()` returns
`"native-chunks-v1"`. Logical root-set projections do not mandate one physical
file per logical chunk. No legacy emitter variant or backend option remains.
Fixture `expectations.backend: "native"` rejects legacy fallback in the unified
runner, separately from input observation and output comparison failures.
Expected runtime failures use normalized names/types and optional codes with
preceding stdout, not unstable stacks/messages; both successful and failing
entries use the same import observer. See the
[fixture schema and current backend counts](../../fixtures/bundler/README.md).
Use an absolute OUTPUT, or note that it is relative to the process cwd.

### Command-line entrypoint

The standalone `bundler-cli` crate owns clap parsing, project/fixture
configuration, JSON reporting and scoped local traces. See its
[README](../bundler-cli/README.md) for configuration and embedding.

## Browser target

Set `options.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser)`.
A target-only switch of untouched default conditions also follows the new host;
nondefault condition sets remain explicit caller policy. Browser defaults activate
`browser`, not `node`/`node-addons`/`module-sync`; package declaration order still
determines exports selection. String `browser` entries and dependency-local
object mappings of the selected legacy entry are supported alongside importer
request/path mappings. No polyfills or empty-module substitutions
are injected. Bare builtin spellings such as `buffer` may resolve real installed
packages through normal checked resolution, including import/require conditions,
exports and cache invalidation. Missing builtin packages and remaining `node:`
requests fail even if listed as externals; installed packages are bundled, not
externalized. Alternatively supply a concrete local stub through
`options.resolver.alias`, for example:

```rust,ignore
options.resolver.alias.insert("fs".into(), "./browser-fs.cjs".into());
options.resolver.alias.insert("node:fs".into(), "./browser-fs.cjs".into());
```

Aliases are exact, one-step replacements: `fs` does not match `node:fs` or
`fs/promises`. Relative replacements resolve from the importer; use an absolute
path to an inventoried file when importers live in different directories.
Explicit replacements precede builtin/external policy and pass through normal
file resolution, checked inventory and module graph loading. Missing files,
native `.node` addons, and replacements still naming `node:` requests or missing
builtin packages fail; no host
`require` or implicit empty stub is supplied. CJS stubs may have bundled
dependencies; ESM stubs used by `require` have the leaf restrictions below.

The browser target emits native ESM chunks for ESM and CommonJS inputs, JSON,
assets and CSS, with conservative Window-host effect analysis. CommonJS
availability chunks export cached lexical initializers; importing availability
does **not** execute a factory. Original `require` sites and demanded ESM facades
activate them. Chains, partial-export cycles, replacement `module.exports`,
throws/retry, finite relative require contexts and static/dynamic ESM interop
share the same factory cache. Context arguments evaluate once; require misses
throw synchronously and dynamic-import misses reject asynchronously, using
captured intrinsics. Browser CJS entries expose native default/named snapshot
exports and default to `.mjs` for `.cjs` inputs; `.js` entries get ESM metadata.
No emitted transport depends on `require`, `module`, `.cjs`, or a Node loader.

Synchronous require of ESM supports dependency-free leaves. Effect-free leaves
(including prepared JSON/asset/caller-loaded global CSS values) keep their native
namespace and may be shared with static/dynamic imports. Effectful leaves must be require-only:
their original body executes at the first require, with cached failures and
read-only getters over live lexical exports. This bridge is not a native
namespace exotic object (its property descriptors are getters). Native shared
jobs for effectful targets, ESM targets with dependencies, and async/TLA targets
are explicitly rejected rather than eagerly executing effects or converting
require to a Promise. JSON/asset/caller-loaded global CSS requires return the
default value; ESM script and other CSS wrapper requires return their namespace.
Browser factories run under native ESM strict mode: use strict-mode-compatible CommonJS, not sloppy-mode
global writes/implicit global `this`. `__filename`/`__dirname` are emitted
availability URLs, not filesystem paths.

Caller-loaded CSS remains application-ordered; auto-mount follows activation
timing. External require, unresolved Node builtins, native addons, unbounded require and loader APIs
(`require.resolve`, `require.cache`, `module.require`) remain unsupported.
Explicit nonbuiltin import externals need a host-provided resolution mechanism.
No worker subtargets, DOM typings, rendering/readiness guarantees, or single-file
runtime are provided. Controlled host-global execution tests cover getter reads,
DOM mutations, scheduling and lazy native imports. Set `BUNDLER_TEST_FIREFOX` to
an installed Firefox executable when running `cargo test -p bundler --lib browser_`
to additionally execute the emitted graph over local HTTP in a real headless
browser, without downloading a browser or using host CommonJS globals. This
checks execution, not rendering/readiness.

## Package-local resolution storage

The private `resolution_cache` backend supplies production `BuildResolver`
requests without changing installed-tree resolution. Checked positive lexical
routes feed demand admission, not arbitrary external canonical-path authority.
`Roots::new` supplies runtime package/project anchors. `CheckedResolution::encode`
accepts complete per-policy request aggregates with original Stage 2 `InputProof`s,
canonical file targets or explicit externals, and package/dependency/policy hashes.
`Store::load_scoped` checks independent typed FlatBuffer schema 1, then freshly validates
borrowed proofs before constructing a checked immutable owner. `find` borrows
requests; stale requests are discarded without evicting other valid requests.
`file_target` reconstructs an anchored path; consulted requests are revalidated
before build success. No JSON parsing or object-tree unpacking occurs on hits.

Package roots contain `.bundler-resolution-v1-<policy-sha256>.fb`; policies do
not overwrite one another. Saving replaces that policy's **complete aggregate**,
not a single-entry overwrite. The resolver aggregates new requests with other
verified compatible requests directly through borrowed wire tables.
`segment_cache::CacheMode` controls reads/writes. Existing `CacheStatus`,
`MissReason` and `CacheWrite` types report disabled/missing/invalid/read failures
and best-effort publication failures explicitly. Sibling tempfile write/flush/
sync/atomic rename never deletes the old record on failure. Windows may deny a
rename while readers hold the destination; this is a diagnosed write failure.

Portable project/package/working-directory anchors distinguish lexical/canonical
and native Windows root spelling,
including local UNC roots; no absolute root strings are persisted. Scoped
project-ancestor anchors cover Node candidates and ancestor `package.json` probes.
Local links inside these anchors retain relative/absolute target form and are
revalidated, including anchor-root retargeting. The working-directory anchor
preserves Stage 2's actual cwd-relative bare-name candidate probes. Arbitrary
external/cross-volume footprints, unsafe/non-UTF8 names,
absolute filesystem request spellings or unproven inputs cannot be persisted;
the resolver conservatively resolves them normally. Reinstallation identity
changes miss even when bytes match. Lockfile fingerprints and policy keys
supplement original actual-request proofs; they never replace filesystem checks.

Focused validation: `cargo test -p bundler resolution_cache --lib`.

## Lockfile identities

The internal `lockfile` adapter supplies production cache identities, not a
public API or authority over `BuildSession` resolution results.
It reads the explicitly chosen canonical project root using upstream
lockfile precedence/selection, returning no graph when no selected lockfile
exists. Format readers avoid upstream's eager local-package manifest filling.
Classic Yarn reads only the root manifest for its direct dependencies; other
workspace/package manifests are read and sealed on demand by `BuildResolver`.

The dependency is the exact published MIT `aube-lockfile = "=2.8.1"`, with no
sibling paths, vendoring or moving Git reference. The upstream release reference
is `a022c6790282f51c1dc84e0309192db480b6df43`; the published crate's
`.cargo_vcs_info.json` records source commit
`963d9b8202ced528d5eb3f72254b896ecf9d10f9`. These are distinct provenance facts,
not an assertion that the registry archive was built at the release reference.
Its Rust 1.91/edition-2024 minimum fits this crate's newer toolchain. Its settings
dependency uses TOML 1 and requires modern Serde: the obsolete SWC-era
`serde <1.0.219` workspace cap was replaced by `serde >=1.0.225, <2`
(SWC is no longer in the workspace dependency graph).

Fingerprints preserve package/source/integrity/peer/patch identities, dependency
names and edge roles. SCC condensation hashing propagates any cycle-member edit
to all affected ancestors without invalidating disconnected packages for
lossless adapters.
Berry additionally retains raw resolved locators, checksums, workspace importer
blocks and named descriptors: upstream's installation-oriented normalization
otherwise merges same-version patches, skips workspace/builtin-patch blocks and
drops unmatched dependencies. Unmatched edges remain represented and mark their
closures incomplete; local/workspace/patch inputs mark closures mutable.
Npm (including shrinkwrap) and Classic Yarn normalization is lossy: unresolved
required/importer declarations are dropped, Classic optional dependencies are
ignored, and npm installations with the same name/version collapse across install
paths, losing context and possibly integrity. **Every package and importer from
these adapters is marked incomplete**, even an apparent leaf. Each identity
includes the entire raw lockfile hash and, for Classic, the root manifest hash,
so edits to dropped declarations or discarded instances remain visible. This
deliberately sacrifices unrelated-package isolation for those formats; it is not
a lossless install-path overlay or installed-tree validation.
Peer declarations without an explicit selected dependency/optional provider
remain incomplete constraints, not
guessed same-name package edges; Yarn's installed virtual peer context is not
invented from its non-virtual lockfile locators. Catalog/transparent-workspace
requests without exact locked descriptors likewise remain explicit incomplete
edges rather than being inferred from all workspace manifests.
Neither flag authorizes reuse without fresh filesystem/manifest validation.
Lockfile entries describe expected installations, not actual Node lookup results.

Focused checks:

```text
cargo check -p bundler --lib
cargo test -p bundler --lib lockfile::tests
```

The ignored `lockfile::tests::actual_project_lockfile` probe accepts
`BUNDLER_LOCKFILE_PROJECT` and reads a local Yarn Berry lockfile without writing
project files or embedding its contents in fixtures. Run it with
`cargo test -p bundler --lib lockfile::tests::actual_project_lockfile -- --ignored --nocapture`.

## Public build session

`build::{BuildSession, BuildOptions, Entry}` joins the existing source cache,
structural resolver, shared graph, effects, exact partition and native linker:

```rust,no_run
use bundler::build::{BuildOptions, BuildSession, Entry};
let mut session = BuildSession::new("project")?;
let mut options = BuildOptions::default();
options.entries.insert("main".into(), Entry {
    path: "src/main.mjs".into(), output_name: None, namespace: true,
});
let result = session.build(&options)?;
let artifacts = &result.product.artifacts;
# Ok::<(), eyre::Report>(())
```

The canonical project root seeds checked demand admission, not a tree walk.
Entries, resolved literal files, CSS/Sass closures and local source maps are
admitted before their bytes are consumed. Membership checks are read-only.
File/directory symlinks and Windows junctions may reach external inputs through
checked lexical routes; a file link authorizes only that file, a directory link
its tree. Canonical paths alone never authorize unlinked external reads. A bare
package resolved from a linked importer may use its exact checked project
`node_modules/<name>` installation edge; no ancestor/sibling tree is enumerated.
Contexts can also use physical projections inside already checked linked trees,
but never discover arbitrary external trees just because a relative pattern
points there. Upstream maps load alongside each prepared script, including new
context candidates; their checked linked scopes join discovery. Settle later
literal/resource/map admissions and their context jobs to a fixed point before
comparing graph edges, independently of worker completion
order. Provide a lexical linked entry rather than an unproven absolute
external entry.

Aliases share one canonical module identity. Outside speculative discovery, only finite contexts enumerate
prefix-compatible scopes, caching each canonical directory listing once per
build and cutting active directory cycles. There is no whole-project fallback
for literal imports. Context checks compare matching named/canonical candidates,
so additions/removals are visible without invalidation from nonmatching files.
Canonical candidates on another Windows volume/share are skipped, retaining
usable lexical aliases. Dangling links/unreadable targets are errors when
actually demanded, not when hidden in unused node_modules subtrees.
External entries require an explicit safe `output_name`; links never authorize
output paths outside owned output. External CSS identity paths use the first
sorted checked in-root route; in-root identities stay canonical/root-relative.
Nearest canonical package metadata determines `.js`
format; explicit extensions override it. Resolution uses the existing resolver's
supported package forms, including bare-string root exports and wildcard access
to arbitrary package files. Export maps still enforce private subpaths.
Custom suffix order uses `BuildOptions.resolver.extensions` with
`prefer_js_extension = false`; otherwise legacy `.js` preference is retained.
The CLI's `resolveExtensions` sets both, accepting optional leading dots and
compound suffixes such as `.web.js`. Omission preserves defaults; `[]` disables
suffix probing without disabling exact-file imports or explicit TypeScript
substitutions. Effective suffix order and preference participate in cache policy.
Actual requests retain positive/negative probes for package trees, link retargets,
priority candidates and deleted targets. Package-format selection seals only
consulted canonical ancestor `package.json` observations, including misses and
linked targets. Sass additionally retains its actual local candidate probes.
Sources, admitted identities/routes, consulted manifests, context matches and
request proofs are rechecked before success, including no-op builds, without
global final scans. Unused JSON and compiler configs are never hashed or probed.
`tsconfig.json`, JSONC comments/trailing commas, `extends` and `references` do not
block unrelated builds. Compiler configs
cannot select package branches or create runtime `baseUrl`/`paths` aliases.
Explicit JSONC resource imports still use the JSONC resource pipeline; removing
compiler-config ingestion does not loosen strict JSON/package metadata parsing.

**Condition migration:** move formerly staged tsconfig
`compilerOptions.customConditions` / `customSourceConditions` into explicit
`BuildOptions.resolver.export_conditions`, for example:

```rust
# let mut options = bundler::build::BuildOptions::default();
options.resolver.export_conditions.push("source".into());
```

For the bundler CLI, pass `--fixture-config FILE` before entries with
`{"resolveConditions":["source"]}`. These additional conditions extend Node
defaults. Previously copied profiling project configs must migrate in the
caller/staging setup; copying tsconfig conditions alone no longer works.
Node itself does not read this custom field: the fixture harness translates it
to explicit `--conditions=source` arguments for source/output observations.
Other Node launchers must supply equivalent flags themselves.
Use package exports or relative requests instead of compiler aliases.
TS/JSX precompilation runs before extraction; existing browser and linked-package
resolution is unchanged.
`.git`, `target`, sidecars and
reserved `.bundler-*` publication directories are excluded.
Source entries/dependencies inside excluded directories are rejected.
Exclusions are scoped to the input root and each checked linked external tree,
not their unrelated absolute ancestors. Every newly admitted tree revalidates
already admitted destinations against its boundary, so a checked outer tree
cannot reveal a reserved-descendant bypass in either admission order. Unvisited
links do not participate in authorization, naming or exclusion scopes.

Source bytes and effective parser config key in-memory and disk parse reuse.
`Fingerprints` independently identifies parse, resolution, effects, entry roots,
fixed Node runtime and emitter/output inputs. Source caches never contain policy.
The session prepares immutable effective Ecco declarations once per configuration
change, sharing the facts and declaration fingerprint across memory checks,
sidecar checks and extraction (no per-source declaration serialization).
Parse, resolution, roots, runtime and output keys are typed XXH3-128
`ast_segmenter::fingerprint::Fingerprint` values rather than 32-byte SHA-256
arrays. The separate effect-policy identity, source/provenance checks,
published-file integrity and content-addressed resource names remain SHA-256.
The session reports actual extraction misses, memory/disk hits, dependency
resolution calls, linker calls and generated-output/recipe parse calls separately.
It passes hash-checked existing owned cache facts/ranges into the same linker,
not a competing IR/pipeline; unchanged sources are **not re-extracted** for
emission. Effect-policy-only changes perform **zero source parsing/resolution**;
resolver-policy changes retain extraction facts but miss incompatible requests.
Generated output is still parsed/semantically validated after transforms.
No-op builds reuse the successful product and perform zero JavaScript parsing,
dependency resolution or emission. Changed/deleted modules'
transitive prior users come from the shared reverse index. Actual edge changes
invalidate their source and prior reverse users, not every inventoried source.
Derived graph/effects/partition/
emission currently rebuild wholesale on any changed key; no fine-grained
later-layer speedup is claimed.

`build` returns `eyre::Result`; failed parsing/resolution/linking/rechecks never
return the prior success. The session retains its last successful product for
later valid builds, not as an error fallback. Cache-write failures are explicit
`warnings`. Sources and consulted inputs are rechecked before success, not
locked against noncooperating editors. A session has one mutable owner and its
shared products use `Rc` (not cross-thread sharing).
Each changed successful snapshot has a monotonic session-local
`BuildProduct::generation`; no-op results share the same product/generation.
Graph-local nodes, handles and plans belong to one immutable product: do not
transfer them between generations or independent sessions.

## Transactional publication

```rust,no_run
use bundler::{build::{BuildOptions, BuildSession, Entry}, publish::{Cancellation, current_directory, prune_stale}};
let mut session = BuildSession::new("project")?;
let mut options = BuildOptions::default();
options.entries.insert("main".into(), Entry { path: "main.mjs".into(), output_name: None, namespace: true });
let output = session.root().join(".bundler-output");
let (build, published) = session.build_and_publish(&options, &output, &Cancellation::default())?;
for warning in build.warnings.iter().chain(&published.warnings) { eprintln!("{warning}"); }
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
Locks explicitly unlock before close (and during unwinding), so fork-inherited
descriptors cannot transiently retain a completed writer's lock.
`build_and_publish` holds that lock across the entire build, rejecting overlapping
builds for the same output before source parsing. Separate `build`/`publish`
calls intentionally permit offline snapshots; sequential snapshot publications
follow commit order, not a source-event freshness ordering.

Errors/cancellation before the pointer commit leave the prior generation and
pointer intact. Cancellation is checked before/after building, per staged file
and before commit, not an interrupt inside parsing. Cancellation arriving after
commit cannot roll back success. Publication is atomic visibility, **not**
directory-fsync/power-loss durability or an adversarial-filesystem transaction.
An OS unlock failure after success is an explicit `Published::warnings` entry,
not a failed transaction that misleadingly implies rollback.
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

## Release gates and measured performance

From the repository root (Node 22, tested v22.22.2; repository Rust toolchain):

```sh
./scripts/bundler-release-check
mkdir -p target/p1-test-tmp
TMPDIR="$PWD/target/p1-test-tmp" cargo bench -p bundler --bench incremental
cargo bench -p bundler --bench cleanup
```

The cleanup benchmark uses managed temporary 130-module projects with one and
four workers. It reports Rust allocation/deallocation counts and bytes alongside
cold/no-op builds, failed attempts, error destruction, session destruction with
and without a surviving product, and final product destruction. Failed-attempt
timings include preparation, not just cleanup. Assertions cover no-op product
identity and product usability after session destruction; these small fixtures
do not predict teardown timings for a large application.

Compared with request-table-only baseline `16f5a50`, three alternating release
runs of this fixture reduced one-worker no-op allocations from about 78,239 to
68,702 (12.2%). Session destruction with a surviving product performed 25,792
deallocations instead of 26,978 (4.4% fewer); failed-session destruction performed
23,558 instead of 23,958 (1.7% fewer). Final product destruction remained at 8,692.
Wall-clock timings were variable; no large-application cleanup speedup is claimed.

The release script tests bundler/parser/resolver/cache/shared graphs/tracker/
good-fences compatibility, builds the public driver, runs both harness regression
suites, native baselines, public-pipeline differential mode, focused all-target
clippy and mandatory `spot-check` (including a workspace **check**, not a full
workspace test suite). Native-only cutover validation:
**442 scoped Rust tests across nine crates (151 bundler)**,
**107 harness regressions**, **32 source/emitted contracts /
44 emitted executions**, same-runtime identity and four diagnostics, focused format,
all-target bundler clippy (`--no-deps`, warnings denied) and mandatory spot-check pass.
**All 32 contracts require/select native; zero legacy/unsupported**:
six esbuild, six rspack and twenty authored cases. Three new Node regressions
exercise ordered multi-owner roots (including CJS), public/private binding-only
roots, export-free evaluation wrappers, empty identities and namespace aliases.
Existing async, CJS activation/retry/mixed-cycle, cached-facts, no-op identity/
mtime and old-generation lazy-loading contracts remain release-gated.
Spot-check additionally reruns 59 ast-segmenter tests and checks the workspace.
Existing unrelated dependency dead-code warnings remain.
The release gate runs the bundler suite in both debug and release profiles,
including cold/warm/no-op/edit parse-count and runtime checks.
The driver exposes planning warnings
separately; the harness reports known effect-order-cycle warnings while still
rejecting cache/publication/unknown warnings.
See the [authored backend baseline](../../fixtures/bundler/README.md#authored-migration-baseline)
and exact historical retirement mapping in NOTES. Independent review of this
native-only deletion increment is pending before committing. No upstream parity
or full workspace test claim is made.

`cargo bench` uses a custom standard benchmark executable, not criterion. Seven
fresh managed project roots per scenario contain a 64-module `export *` chain,
an entry and a conditionally exported package with a package-local reexport
(**67 reached sources / 66 requests**), with a Berry lockfile and explicit
installed locator.
Each sample runs cold parse-cache → new-session warm disk cache → same-session
no-op → one local declaration edit → one added public export → package manifest
edit → explicit resolver condition switch. Before no-op it also changes a
disconnected lockfile package and starts another fresh session: all 66 requests
must still hit, with identical artifacts and no publication writes.
The package manifest edit must miss only two requests while retaining 64 hits.
Timings include actual source/config IO, the full public pipeline and
transactional publication; compilation, fixture setup and runtime execution are
outside the timed section. “Cold” means no source sidecars/session/output,
**not** flushed OS caches. Warm output already exists. Source/derived caches are
not faked; assertions verify actual public counters, including resolution
hits/misses and zero inventory directory reads on cold literal builds.
Cache byte totals include both extraction and package-local resolution sidecars.

Historical cached-source migration release measurements, after the audit
prerequisites (the config-edit row used the now-removed tsconfig condition input,
not the current explicit-options benchmark):

| Scenario | Before median total ms | After median total ms | After min–max ms | After build ms |
| --- | ---: | ---: | ---: | ---: |
| Cold | 77.668 | 81.220 | 80.303–83.157 | 80.991 |
| Warm | 74.931 | 78.823 | 75.996–82.620 | 78.671 |
| No-op | 2.929 | 4.125 | 4.021–5.256 | 3.981 |
| Local edit | 71.491 | 72.889 | 69.862–75.672 | 72.768 |
| Export edit | 75.577 | 76.067 | 73.581–77.662 | 75.804 |
| Config edit | 74.686 | 75.779 | 74.221–77.431 | 75.545 |

| Scenario | Extractions | Resolutions | Link calls | Source/fragment reparses | Release final validations | Artifact writes | Cache bytes before / after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Cold | 66 | 65 | 1 | 0 / 0 | 0 | 1 | 56,308 / 110,230 |
| Warm | 0 | 65 | 1 | 0 / 0 | 0 | 0 | 56,308 / 110,230 |
| No-op | 0 | 0 | 0 | 0 / 0 | 0 | 0 | 56,308 / 110,230 |
| Local edit | 1 | 1 | 1 | 0 / 0 | 0 | 0 | 56,427 / 110,590 |
| Export edit | 1 | 0 | 1 | 0 / 0 | 0 | 1 | 56,611 / 111,088 |
| Config edit | 1 | 65 | 1 | 0 / 0 | 0 | 1 | 57,259 / 112,312 |

Warm has 66 disk hits; no-op has 66 memory hits; each edit has 65 memory hits.
Artifact writes count actual staged files, excluding atomic pointer metadata.
This eligible single-root fixture has **one native chunk and one total file**.
Changes still stage a complete immutable generation. The previous public pipeline
performed 135 emission parses per changed build; release now performs zero.
Debug builds validate this one final executable once, never on no-op.
`Counters` and the example JSON report extraction, source reparses, fragment
reparses and final validations separately; `output_parses` remains the total
emission-parse compatibility field. Source re-extraction is measured from actual
module-parser calls, not inferred from cache hits. Cache bytes count source
sidecars and are measured outside the timed interval.
The measurements apply only to this synchronous ESM benchmark, not every graph shape.
Eliminating parses did not improve these end-to-end timings: owned recipes and
provenance checks increase cache size and checking work. These are local,
hardware-dependent samples, not a speedup claim.

This is a correctness-first baseline, not fine-grained later-layer incrementality:
config changes re-resolve all requests and any derived key change rebuilds the
whole graph/effects/partition/output, while source parsing stays selective.
Demand admission and success checks inspect consulted inputs only; literal
builds do not walk the project tree. Ownership checking
and debug-only final output validation remain whole-product operations.
No heuristic chunk optimization, worker scaling or large-monorepo speed claim.

**Limits:** Linux was executed; Windows atomic replacement/OS locks rely on
standard-library/tempfile contracts but were not runtime-tested. No
standalone single-file output, assets/native addons, executable
externals or import.meta. Optional JavaScript source maps are described above.
The exact root sets are the existing
conservative retention contract, not a whole-program minimality claim. CJS bridge
semantics intentionally differ from native namespace exotica as documented below.
Old-reader quiescence is mandatory before pruning immutable prior generations;
publication promises atomic visibility, not power-loss durability.

## CommonJS runtime

ESM inputs may read `module`/`exports` or probe `typeof require` for environment
detection (including lodash-es guards). These stay native free references, not
CommonJS wrapper bindings: the bundler does not inject Node globals or replace
guards with `false`. Host-provided values/getters, short-circuit timing, lexical
shadowing/TDZ and unguarded ReferenceErrors remain observable. Direct wrapper
writes, deletes, updates, calls, construction and tags remain unsupported in ESM;
aliases remain ordinary JavaScript values, not inferred loaders.
Direct unshadowed literal `require()` calls are supported in ESM without changing
its format. Ordinary and reconstructed recipes share whole-call expression
planning: availability imports do not invoke CJS factories; the original call site
owns cached activation. Browser ESM/resource targets follow the synchronous leaf
and shared-default rules above. ESM require contexts, loader aliases/mutations and
external requires remain rejected. On Node, mixed ESM may require bundled CJS
(including prepared JSON), but synchronous ESM targets remain unsupported there.
Unused native `module`/`exports` reads and `typeof require` retain their read
effects through Ecco refinement, including host getters and missing-global errors;
this includes unused calls to functions and indirect helpers that perform reads.
Uncalled probe functions do not disable refinement. Executed host reads use Ecco's
opaque-execution widening, which can also retain otherwise pure calls whose
globals or values might be mutated by a getter.

Package defaults select the initial format. Static ESM syntax disambiguates
CommonJS-default `.js`/`.jsx`/`.ts`/`.tsx`; explicit m/c extensions retain their
format. Environment probes and direct require calls never switch effective ESM
into a CommonJS wrapper.

Linked opaque JavaScript CJS bodies appear once as real function syntax in pure
availability chunks (`.cjs` on Node, native `.mjs` on Browser).
These chunks group factories by existing root sets;
they are not copies of input modules that Node executes/caches as source.
Each exported initializer owns one lexical record, allocated before invoking its
factory. Source requires call checked initializer bindings directly, including
across availability chunks. On Node, requires of synchronous ESM remain lazy native
requires of the actual ESM chunk path. Browser's bounded synchronous ESM support
is described above.
The initializer binds wrapper `this` to the
initial exports object and returns the current `module.exports`. Recursive
requires see partial exports; deferred sites execute only when called. Failed
factories are removed from the require cache and retry on the next require.
Successful modules execute once. Native ESM proxies carry static named/default
requests; CJS namespace requests/entries use a canonical runtime-key bridge.
`__filename` and `__dirname` are factory-local wrapper parameters initialized
from the actual output availability chunk's native path bindings on Node (the CJS
counterpart of `import.meta.filename`/`import.meta.dirname`). They describe emitted
files, not original source locations. `var` redeclarations retain those values;
writes stay local to one factory, and nested lexical shadows remain ordinary JS.
Top-level lexical redeclarations conflict with wrapper parameters and are rejected
with source diagnostics, as in Node.

When CJS is retained, `native/runtime.cjs` (Node) or
`native/browser-intrinsics.mjs` (Browser) captures generated-shim intrinsics
before application evaluation. Every emitted job loads it first, including
otherwise unrelated entries, so late namespace jobs and public CJS aliases reuse
the same captures. Namespace construction, factory invocation and cycle errors
never look up public Object/Reflect methods again. Namespace key traversal uses
indices rather than the mutable array iterator. Application code keeps its own
ordinary built-in lookups unchanged. Pure ESM builds need no helper.
The support file is included in every logical chunk's artifact projection;
it contains no source loader, factory registry or application initialization.
Captures reflect the host at the bundle's first load, not a pristine realm if
unrelated code already patched it.

### Interop policy

`Artifacts::cjs_proxies` reports relative executable proxy paths and static surfaces.
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
`Artifacts::cjs_namespaces` reports separate native jobs whose default
exports are canonical CJS namespace bridges. Static `import * as ns`,
`export * as box`, and rewritten dynamic CJS imports use the same bridge object. Its own string
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
Proxies and requires share the same initializer record; no duplicate factories.
ESM interop cannot import a still-running CJS factory, retaining Node 22's
`ERR_REQUIRE_CYCLE_MODULE`; CJS requires can still read its partial exports.

### Logical proxy integration

`chunks::NodeId::CommonJsProxy { module, name }` participates in ordinary exact
reachability and partitioning: `None` owns initialization/default and `Some(name)`
owns one named snapshot. `NodeId::CommonJsNamespace(path)` owns the conservative
runtime-key bridge. Consumers demand their own names; namespace consumers and
CJS entry facades demand that bridge, not a finite name list. Requires demand the opaque
body/target namespace, not a proxy or eager execution. Manifest synthetic nodes
and chunk dependencies expose proxy→factory and snapshot→initialization edges.
Every implementation has one physical owner, but logical partitions need not
be files. Adjacent default/named parts with equal consumer evaluation seams share
a native ESM job; other names keep separate jobs so a late consumer cannot
resnapshot an earlier name or trigger an unsolicited getter. Unused import
specifiers do not demand named getters even beside live specifiers. Namespace
bridges keep their own canonical default-export job. Static dependencies load
initialization/sorted demanded snapshots in source dependency order.
`cjs_proxies` points directly to a complete native static surface where possible;
only split surfaces need an aggregate reexport boundary. Ordinary imports load
their own requests, not that aggregate. `cjs_namespaces` points directly to
the bridge job; static/dynamic/reexport-as consumers use that same bridge.

All routes share lexical initializer records and canonical native interop jobs.
Only public CJS entries need body-free `.cjs` format wrappers, preserving legal
CJS-first mixed cycles without turning them into evaluating-ESM require errors.
Their exports accessor forwards to the initializer's current result, so repeated
native requires of a public path observe later `module.exports` replacement.
Importing a CJS entry directly uses Node's native CJS interop; use the path in
`cjs_namespaces` and its default export for the bundler's runtime-key bridge.
Opaque require of synchronous ESM conservatively
retains the whole target namespace. Native Node owns sync require-of-ESM behavior:
completed/deferred mixed cycles work, while requiring an evaluating ESM in a
cycle throws `ERR_REQUIRE_CYCLE_MODULE`. Require of async ESM is rejected before
emission, even for deferred require sites; use dynamic import instead.
All static and dynamic routes use ordinary relative file paths.

There is no generic ESM registry/runtime or per-input `.cjs` fallback. Only the
CJS-specific factory/cache and namespace bridge implement the interop policy.

## Rewrite backend

`rewrite::{Edit, apply, validate, rename_private}` uses checked original-source
byte ranges and debug-validates the final result with the workspace-pinned OXC
parser and semantic checker. UTF-8 boundaries, overlapping replacements and duplicate
insertions are errors. Private lexical renames use owned binding identities,
not textual matching; object/binding shorthand expands to preserve keys.
Capture and public-binding renames are rejected rather than guessed.
Unedited comments, directives and hashbangs remain byte-for-byte unchanged.

At OXC 0.155.0 (revision `73e6b564737a29ecebf428d276421dcc492836df`),
`Codegen::build(&Program)` prints a whole arena-owned AST rather than preserving
the original source layout. Source edits fit our owned occurrence
spans without reconstructing/rebinding an AST. After optional TypeScript/JSX
precompilation, linking therefore uses edits, not the printer/transformer.
Low-level `emit::single_module`, `link::esm` and `BuildSession`
share source/configuration/facts provenance verification without reparsing.
Structured fragments preserve original occurrence identities through pruning and
destination selection. Debug builds parse/semantically check final output once;
release builds rely on always-on provenance, edit, identifier and target checks.
Output is JavaScript ESM/CJS only. The emitter rejects raw TypeScript/JSX;
`BuildSession` lowers those inputs before extraction. Optional BuildSession
source maps preserve copied-source/renamed-identifier origins through assembly
and compose upstream mappings after final artifact destination selection.
Cached segment recipes remove stale in-segment `sourceMappingURL`/`sourceURL` directives; emission
does not mistake matching string literals or ordinary comments for directives.
Diagnostics for rejected edits refer to original source bytes.

## Single-module emission

`emit::single_module(graph, retention, policy, file_id, source)` returns owned
JavaScript without writing files. Supply a current retention plan; for public
exports, request their checked binding routes or the module namespace when
building retention. Evaluation-only entrypoints need not retain unused exports.
The emitter checks source provenance and owned facts/ranges against the graph,
removes dead statements and instantiates cached recipes selected by retention.
Final output is parser/semantic checked only in debug builds. Native ESM preserves lexical scopes, hoisting, TDZ,
live bindings and default/alias exports; no wrapper snapshots are introduced.

PURE erasure evaluates callee/reference and arguments in order, consumes spread
iterators, preserves optional-chain completion separately from an undefined
getter value, and composes nested recipes. Fresh arrow parameters avoid capture.
Guarded invocations remain intact when lowering would move top-level `await`
across a generated arrow boundary; making the arrow async would change timing.
Await in an unguarded receiver and awaits inside existing async functions do not
disable otherwise safe PURE erasure.
Optional calls/private fields **inside an optional PURE callee chain** are
explicitly rejected; ordinary optional terminal calls are supported.
The current single-module API rejects runtime dependencies (use linked emission),
CJS/TS/JSX modes, and source-policy unsupported syntax such as import.meta.
Source TLA/for-await remains native ESM syntax, including suspension timing.
No externals, implicit TS stripping, source maps or filesystem publication.

## Linked ESM artifacts (Node target)

`link::esm(graph, &ChunkPlan, policy, &BTreeMap<PathBuf, String>)` returns
`Artifacts` containing the selected `backend`, complete owned relative `files` map, executable
`entries`, source-to-executable `modules`, logical-node-to-file
`members`, `chunk_artifacts`, and the unchanged
logical diagnostic `manifest`. It checks freshness and owned source facts,
uses P4 retention/PURE recipes, and rewrites checked static import/re-export
routes. Unused import bindings/re-exports disappear when their statement-level
retention permits it; their dependencies still evaluate. Star ambiguity,
explicit precedence, default exclusion and namespace/live aliases remain ESM
semantics, not object snapshots.

**Target: normal Node 22 (tested v22.22.2).** Publish the entire returned map
without changing its relative layout. Run `node <artifacts.entries[name]>`
or import that file directly:

```js
import value, {namedExport} from './native/<entry-path-hash>.mjs';
```

Namespace entries and ESM dynamic targets point to their actual body chunks (the
single-tree fast path uses `native.mjs`), not namespace-object launchers.
ESM exposes actual static named/default exports;
CJS executes directly as CJS. Request namespace binding demand when all public
exports must remain; evaluation-only
or selected-binding roots intentionally do not retain a complete namespace.
Repeated/concurrent entries and static namespace re-exports share one canonical
native identity per module. Separate published bundle directories have isolated
native identities.

`members` names actual owners; every retained original body is emitted once.
Logical `chunk_artifacts` projections may overlap or contain multiple files.
Public wrappers are real native boundaries, not pretend original body owners.
The native public export surface contains no private setter/capture exports.
No code JSON, hooks, loader registration, preload, custom protocol, source-text
IO, source assembly or runtime compilation is used.

Node owns native dependency/SCC scheduling, public namespaces and async jobs.
Low-level roots naming multiple modules emit evaluation-only wrappers without invented
namespace merges or exported arrays. `link::esm` itself writes no files.

Full literal source dynamic imports use the same native identities as static
imports and entries. Selected occurrences instead project native live bindings
without exposing a full namespace. `Artifacts::async_facades` maps each reached
source occurrence to its native projection, canonical ESM target or CJS bridge job;
ESM imports add no promise-unwrapping
reaction. Native import promises preserve source suspension and
deduplicate module evaluation. CJS imports unwrap the existing namespace bridge
with a promise continuation (one extra reaction, not a synchronous require).
Nested sites stay deferred; unreachable sites emit nothing. File resolution,
parse failures and retry follow Node's native loader behavior (not a separate
bundler cache); evaluation errors remain cached by native ESM. Changed artifacts produce a
new immutable published generation; unchanged content intentionally keeps the
same directory/native jobs, never an in-place repair of failed jobs.
Externals, TS/JSX and other source-policy unsupported forms still fail with
diagnostics. Logical root-set equality remains exact; native physical grouping
does not change that analytical partition.
Public browser builds support native ESM transport and lexical CommonJS
initializers as described above; this is not a standalone-file target.

### Async evaluation and support

The native backend retains original source TLA/top-level for-await syntax in
body-bearing chunks. Async source modules and their static consumers keep native
job boundaries; compatible synchronous subgraphs still concatenate. No generator
events, binding cells or async wrappers are introduced. Node's native module jobs
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

| Source construct | Native Node 22 linked target |
| --- | --- |
| Literal dynamic ESM import | Native namespace promise, shared evaluation |
| Literal dynamic CJS import | Shared frozen runtime-key bridge; extra promise reaction |
| ESM TLA / top-level for-await | Native host dependency/SCC scheduling and rejection |
| Synchronous require of sync ESM | Native namespace and evaluating-cycle errors |
| Require of async ESM (including static transitive dependencies) | Source diagnostic before emission |
| CJS top-level await / for-await | Parse diagnostic; opaque bodies stay synchronous |
| Unconstrained/bare nonliteral imports/requires, options/attributes, assets, import.meta | Explicit unsupported gate; no silent omission |

The unified public-build Node runner compares all 37 portable cases (52 entry executions)
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
binding roots/edges expand checked namespace export routes; module
evaluation alone does not imply namespace demand. This oracle describes code
availability, not runtime evaluation, especially for deferred synchronous require.

`chunks::partition(graph, entries, policy)` accepts only `RootId::Entry` seeds.
It recomputes existing P4 retention as reachable async sites activate independent
checked export roots (or conservative namespace roots), to a fixed point including
nested sites/cycles. `partition_conservative` is the explicit namespace oracle.
External async sites
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
files are emitted). `entries` and `async_facades` hold logical facade records,
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
The documented ESM/CJS, dynamic-import and TLA subset has emitted-runtime evidence.
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
