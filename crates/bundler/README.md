# bundler

## Replacement direction and current status

The symbol linker uses scope-hoisted direct ESM bindings and native
imports/exports only across actual output chunks. Root sets remain reachability
analysis, not mandatory physical partitions. Public entry/export boundaries
constrain output; pure segments may move according to binding dependencies.
Each CJS input contributes one opaque segment plus a logical ESM facade, both
subject to normal usage/linking/grouping, not a native `.cjs`-per-input fallback.

The registration/cell/alias/subscriber/generator-dispatch/capture-cycle backend
is **deleted**, including its runtime and fingerprints. All linked output is
native; there is no fallback or feature flag.
The internal `symbols::Symbols::link` builds checked canonical
`(file, BindingId)` definitions and import/export routes using the shared graph
resolver. Native emission reuses that plan, preserving intermediate barrels and
every retained var declaration.
Lexical names are allocated in path/declaration order, avoiding nested bindings,
free references, keywords and helpers. Observable function/class/inferred names
remain separate from those allocations. Namespace,
default-expression/anonymous-declaration and opaque CJS values retain typed graph
identities, not invented lexical binding IDs; checked syntax still owns default
hoisting/initialization semantics. No public product field or dependency was added.

## Native ESM chunks

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
static-import DFS order, never path-sorted body order. PURE recipes compose before
owned occurrences are re-extracted and checked OXC-aware edits are applied.
Shorthand keys, free globals, nested shadows, hoisting, strict-module `this`,
TDZ, const and live mutation keep native semantics. The output is parser/semantic
validated and has no runtime, cells, registrations or per-module facade family.

Public `BuildOptions.entries` keys are logical IDs. `Entry.output_name` is an
optional relative output name/path supplied by the build config. When omitted,
the output preserves the canonical source path relative to the project root:
input `"./entry.mjs"` emits `entry.mjs`, regardless of its entry ID.
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

The evaluation planner merges an exclusively consumed dependency group when
it is the consumer group's only remaining dependency. It can also absorb
body-free import/reexport connectors among siblings. Unobserved pure barrels
redirect to their dependency only when their complete export-name/canonical-
identity maps match, including `default`; subset surfaces must not widen later
star exports or introduce ambiguity. Source-ordered static DFS
then preserves dependency completion and statement order; identical logical root
sets alone never authorize moving effects. Shared dependencies remain once-only
body owners. Public entry surfaces and namespace targets keep precise native
surfaces; internal imports/exports disappear inside a merged group. No generated
private names leak into namespace keys.

Physical owners are independently assigned to retained segments, not input
modules or root-set partitions. Pure lexical components move to a unique consumer
or independent shared binding chunks; components from unrelated sources can
co-locate. Eager local initializer reads and writes keep their lexical components
together, preserving snapshots and mutable closures without cells or setters.
Name-sensitive components split into separate chunks when co-location would
change names or capture references. Consumer rewrites and artifact members follow
actual segment owners, including aliases and default-expression addresses.

Namespaces and CJS-consuming modules no longer impose whole-module placement
barriers. Namespace demand retains checked visible exports and ordinary effects,
not unrelated private declarations. Empty private evaluation chunks are removed,
their evaluation imports are spliced in original order, and no-op metadata owners
are remapped to surviving consumers. Public, escaped and dynamic namespace
boundaries remain native modules with exact keys and identity. Cyclic connector
jobs remain where splicing would bypass a still-evaluating job's dependency order.

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

Dynamic namespace targets retain their complete public surfaces, without private
exports. Source TLA status propagates over static runtime dependencies, including
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
The 66-source star-chain benchmark
still asserts one ordinary public `main.mjs`, without runtime/helpers.

## Retention

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
`cargo build -p bundler --example build && node scripts/bundler-fixtures.mjs`.
Harness regressions:
`node --test scripts/bundler-fixtures.test.mjs scripts/bundler-fixtures-build.test.mjs`.
The runner validates the entire corpus before execution, skips builds for bad
inputs, and collects input/build/required-backend/output failures without stopping
later cases. It reports native and legacy passes separately; unexpected failures
make the CLI exit nonzero after reporting all cases. The same command retains
the same-runtime identity and source-range diagnostic checks.
The minimal driver can also build a project directly:
`cargo run -p bundler --example build -- PROJECT OUTPUT ENTRY...`; it prints
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

The canonical project root is a bounded dependency/configuration boundary.
All resolved sources must remain inside it. In-tree symlinks (including directory
symlinks) are explicitly rejected by this orchestration API; the lower-level
resolver retains its separate policy. Nearest package metadata determines `.js`
format; explicit extensions override it. Resolution uses the existing resolver's
supported package/tsconfig forms (not every Node resolver feature; e.g. bare
string `exports` is currently rejected). JSON/JSONC bytes and all file names
under the root conservatively invalidate resolution, including package trees,
config ancestry, new files and deleted targets. The public API gates unsupported
tsconfig forms instead of silently ignoring them: use strict `tsconfig.json`
JSON with direct `compilerOptions.baseUrl`/`paths`; no extends/references,
overlapping mappings, multiple wildcards or nonterminal wildcard patterns.
Other TypeScript compiler options are not runtime transforms.
`.git`, `target`, sidecars and
reserved `.bundler-*` publication directories are excluded. Keep unrelated large
trees outside the project root; this is measured coarse inventory, not a watcher.
Source entries/dependencies inside excluded directories are rejected.

Source bytes and effective parser config key in-memory and disk parse reuse.
`Fingerprints` independently identifies parse, resolution, effects, entry roots,
fixed Node runtime and emitter/output inputs. Source caches never contain policy.
The session reports actual extraction misses, memory/disk hits, dependency
resolution calls, linker calls and generated-output/recipe parse calls separately.
It passes hash-checked existing owned cache facts/ranges into the same linker,
not a competing IR/pipeline; unchanged sources are **not re-extracted** for
emission. Policy-only changes perform **zero source parsing/resolution**.
Generated output is still parsed/semantically validated after transforms.
No-op builds reuse the successful product and perform zero JavaScript parsing,
dependency resolution or emission. Changed/deleted modules'
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
```

The release script tests bundler/parser/resolver/cache/shared graphs/tracker/
good-fences compatibility, builds the public driver, runs both harness regression
suites, native baselines, public-pipeline differential mode, focused all-target
clippy and mandatory `spot-check` (including a workspace **check**, not a full
workspace test suite). Native-only cutover validation:
**442 scoped Rust tests across nine crates (151 bundler)**,
**99 harness regressions**, **30 source/emitted contracts /
41 emitted executions**, same-runtime identity and four diagnostics, focused format,
all-target bundler clippy (`--no-deps`, warnings denied) and mandatory spot-check pass.
**All thirty contracts require/select native; zero legacy/unsupported**:
six esbuild, six rspack and eighteen authored cases. Three new Node regressions
exercise ordered multi-owner roots (including CJS), public/private binding-only
roots, export-free evaluation wrappers, empty identities and namespace aliases.
Existing async, CJS activation/retry/mixed-cycle, cached-facts, no-op identity/
mtime and old-generation lazy-loading contracts remain release-gated.
Spot-check additionally reruns 51 ast-segmenter tests and checks the workspace.
Existing unrelated dependency dead-code warnings remain.
The driver exposes planning warnings
separately; the harness reports known effect-order-cycle warnings while still
rejecting cache/publication/unknown warnings.
See the [authored backend baseline](../../fixtures/bundler/README.md#authored-migration-baseline)
and exact historical retirement mapping in NOTES. Independent review of this
native-only deletion increment is pending before committing. No upstream parity
or full workspace test claim is made.

`cargo bench` uses a custom standard benchmark executable, not criterion. Seven
fresh managed project roots per scenario contain a 64-module `export *` chain,
an entry and a tsconfig-selected dependency (**66 reached sources / 65 requests**).
Each sample runs cold parse-cache → new-session warm disk cache → same-session
no-op → one local declaration edit → one added public export → tsconfig target
switch. Timings include actual source/config IO, the full public pipeline and
transactional publication; compilation, fixture setup and runtime execution are
outside the timed section. “Cold” means no source sidecars/session/output,
**not** flushed OS caches. Warm output already exists. Source/derived caches are
not faked; assertions verify actual public counters.

Native-only cutover measurement on Linux 7.2.8 x86_64, AMD Ryzen 7 7840U (8 cores / 16 threads),
Rust `1.97.0-nightly (f53b654a8 2026-04-30)`, Cargo `1.97.0-nightly`, bench
profile optimized + debuginfo; Node conformance runtime v22.22.2:

| Scenario | Median total ms | Min–max ms | Build portion at median ms |
| --- | ---: | ---: | ---: |
| Cold | 92.947 | 90.797–109.781 | 90.512 |
| Warm | 43.016 | 42.557–49.721 | 42.886 |
| No-op | 3.072 | 3.006–3.643 | 2.921 |
| Local edit | 41.407 | 41.232–50.829 | 39.483 |
| Export edit | 43.833 | 41.921–46.973 | 42.009 |
| Config edit | 44.284 | 43.355–47.283 | 42.109 |

| Scenario | Source parses | Resolutions | Link calls | Output/recipe parses | Artifact writes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cold | 66 | 65 | 1 | 134 | 1 |
| Warm | 0 | 65 | 1 | 134 | 0 |
| No-op | 0 | 0 | 0 | 0 | 0 |
| Local edit | 1 | 1 | 1 | 134 | 1 |
| Export edit | 1 | 0 | 1 | 134 | 1 |
| Config edit | 1 | 65 | 1 | 134 | 1 |

Warm has 66 disk hits; no-op has 66 memory hits; each edit has 65 memory hits.
Artifact writes count actual staged files, excluding atomic pointer metadata.
This eligible single-root fixture has **one native chunk and one total file**.
Changes still stage a complete
immutable generation. Each reached module has owned-occurrence and syntax parses;
the one output has source-hint and validation scans: 2 × 66 + 2 = 134.
Those cutover measurements predate entry-output naming. The current public
pipeline adds one request-rewrite scan: 135 output parses per changed build,
still zero on no-op and one emitted file named `main.mjs`.
The measurements apply only to this synchronous ESM benchmark, not every graph shape.
Cached source-fact reuse still avoids redundant emitter source re-extraction.
No timing improvement percentage is inferred from these small hardware-dependent
samples. Evidence: `target/native-only-bench.log` (not a committed artifact).

This is a correctness-first baseline, not fine-grained later-layer incrementality:
config changes re-resolve all requests and any derived key change rebuilds the
whole graph/effects/partition/output, while source parsing stays selective.
Inventory and success rechecks walk the bounded project twice. Generated-output
validation and ownership verification remain whole-product operations.
No heuristic chunk optimization, worker scaling or large-monorepo speed claim.

**Limits:** Linux was executed; Windows atomic replacement/OS locks rely on
standard-library/tempfile contracts but were not runtime-tested. No browser,
standalone single-file output, TS/JSX lowering, assets/native addons, executable
externals, import.meta or source maps. The exact root sets are the existing
conservative retention contract, not a whole-program minimality claim. CJS bridge
semantics intentionally differ from native namespace exotica as documented below.
Old-reader quiescence is mandatory before pruning immutable prior generations;
publication promises atomic visibility, not power-loss durability.

## CommonJS runtime

Linked opaque JavaScript CJS bodies appear once as real function syntax in pure
availability `.cjs` chunks. These chunks group factories by existing root sets;
they are not copies of input modules that Node executes/caches as source.
Each exported initializer owns one lexical record, allocated before invoking its
factory. Source requires call checked initializer bindings directly, including
across availability chunks. Requires of synchronous ESM remain lazy native
requires of the actual ESM chunk path, not eager ESM imports.
The initializer binds wrapper `this` to the
initial exports object and returns the current `module.exports`. Recursive
requires see partial exports; deferred sites execute only when called. Failed
factories are removed from the require cache and retry on the next require.
Successful modules execute once. Native ESM proxies carry static named/default
requests; CJS namespace requests/entries use a canonical runtime-key bridge.
`__filename` and `__dirname` are factory-local wrapper parameters initialized
from the actual output availability chunk's native path bindings (the CJS
counterpart of `import.meta.filename`/`import.meta.dirname`). They describe emitted
files, not original source locations. `var` redeclarations retain those values;
writes stay local to one factory, and nested lexical shadows remain ordinary JS.
Top-level lexical redeclarations conflict with wrapper parameters and are rejected
with source diagnostics, as in Node.

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
printer/transformer. Low-level `emit::single_module` / `link::esm` reparse source
to check caller-supplied graph facts. `BuildSession` instead reuses its own
hash-checked validated source-cache records. Both paths parse/semantically check
generated output; source facts alone cannot validate transformed code.
Output is JavaScript ESM/CJS only: no TypeScript or JSX
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

Literal source dynamic imports use the same native identities as static imports
and entries. `Artifacts::async_facades` maps each reached source occurrence to
its canonical executable target path (native ESM body chunk or CJS bridge job);
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
This backend is not a browser/standalone-file target.

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
| Nonliteral imports/requires, options/attributes, externals/assets, import.meta, TS/JSX lowering | Explicit unsupported gate; no silent omission |

The unified public-build Node runner compares all 30 portable cases (41 entry executions)
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
