# bundler

Consume owned-or-wire syntax views and checked shared graph edges, never UsedTag.
Keep immutable AST/Ecco owners alive through graph and worker reads; never
unpack cached facts for linking, validation, execution constraints or recipes.
Solve cached Ecco precursors on the complete checked graph, using canonical export
identities; never cache solved effects. Share a solve across async partition rounds.
Declaration structs key source metadata, memory reuse and parse fingerprints.
Keep one validated immutable `PreparedDeclarations` per session configuration;
share its fingerprint through memory/disk checks and extraction. Internal build
layer keys use typed XXH3-128 fingerprints; publication/content integrity and
resource naming retain SHA-256.
Select Ecco host contracts from resolver target and key preparation by host plus
declaration config; retain explicit user overrides and wrapper read contracts.
Browser emits native ESM only, including lexical lazy CJS availability,
JSON/asset/global CSS wrappers and finite require/dynamic-import contexts.
Availability imports never invoke factories; require sites/facades own activation.
Resolve explicit one-step exact aliases before rejecting Node builtins even as
externals; local stubs still require checked inventory/graph loading. Never
provide automatic polyfills/empty stubs; reject native addons and external require.
Deduplicate availability imports by output chunk and binding, not factory.
Capture shim
intrinsics in native ESM before entry effects, never package a Node loader.
Browser synchronous ESM require supports dependency-free leaves: effect-free
leaves retain native shared namespaces; effectful require-only leaves use cached
lexical initializers/read-only live getters. Reject shared effectful jobs,
dependent ESM and async boundaries, never eagerly shift their effects.
JSON/asset/caller-loaded global CSS requires unwrap prepared default values,
selected by the build's resource policy, never inferred from source extensions.
Browser CJS entries use ESM
snapshot facades; wrapper path parameters are availability URLs.
Default condition lists follow target switches; custom lists stay caller policy.
Keep conservative initializer/effect ordering and add-only forced-effect rules.
TDZ-error preservation is waived; other binding and write restrictions remain.
BuildSession is the public orchestration boundary: reuse source-local cache facts,
separate layer fingerprints, inventory config/add/delete changes, and derive
affected users from the existing reverse index. Never return old success on error.
Entry IDs are logical labels. Entry.output_name optionally overrides the
source-relative output path; include it in output invalidation, not root identity.
Infer portable entry names from host path components; explicit output names stay
slash-only. Publication validates components, not host separator spellings.
Convert canonical source-relative filesystem paths to slash-separated artifact
names; retain portable separators in output metadata, including package.json.
Validate explicit output spelling before normalization; never accept backslashes.
Renaming canonical files must rewrite requests and artifact routes together.
Keep generated requests as typed artifact identities until empty-job pruning and
destination/alias selection finish; never recover requests by parsing output.
Later-layer invalidation is deliberately coarse; counters separate extraction,
source/fragment reparses and final validation. Emission reparses must stay zero;
debug validates final executable files once, release never does, no-op does neither.
Reuse hash-checked cache records
in session emission; public low-level link::esm shares its parse-free
source/config/facts provenance checker. Analysis-only graph inputs cannot emit.
Input inventory follows reachable symlinks/junctions, including external packages,
with canonical source identity. Scan each canonical directory once and retain
named directory edges, not exponentially many paths through alias diamonds.
Admit only inventoried sources; never enumerate unrelated ancestor trees.
Scope reserved-directory exclusions to input/linked tree roots, never unrelated
absolute ancestors; validate canonical destinations against all discovered roots
after traversal so cross-tree aliases cannot bypass exclusions in either order.
Track canonical targets, membership, JSON bytes and existing/missing ancestor
package/tsconfig probes for invalidation and success rechecks.
External inventories refresh resolution each build and replay actual requests
before success; this detects ancestor node_modules shadowing without broad scans.
Contexts expand prefix-compatible finite non-cyclic alias paths on demand,
including physical ancestor projections; omit cross-volume relative candidates
without losing same-volume lexical aliases.
External CSS identities use the first sorted in-root alias; in-root identities
remain canonical/root-relative. External entries require explicit safe output
names; input links never relax publication/output path checks.
Product generations are session-local immutable snapshots; never transfer
graph-local handles/plans between products. No-op retains the same generation.
Never discover/parse/validate tsconfig for resolution, including extends/references.
Use explicit BuildOptions.resolver.export_conditions; never ingest
customConditions/customSourceConditions or rewrite paths/baseUrl.
Inventory may hash compiler config bytes without interpreting them.
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
`RetentionPlan::execution` is scheduling-only and shares revision/policy validity.
Mandatory initialization/pre-init/read-write constraints never use effect-SCC
breaking. Direct linked reference/declaration relationships bypass barrels without
inventing cross-source declaration-before-read edges. Located barriers fence the
owning native source phase until proven safe, including other mutable accesses;
neither deferred access nor inferred purity authorizes eager execution.
Use original eager syntax-effect sites for implicit execution/throw fences,
independent of PURE/Ecco. Mandatory AbruptCompletion constraints preserve both
sides of each source-local phase, including unrelated lexical initializations;
do not order independent entries or move TDZ declarations ahead of failing reads.
Resource preparation separates original input bytes from generated JS text and
parser paths. Graph paths retain input identity; spans/provenance describe only
generated text. Recheck original bytes on reuse and return. JSON uses a shared
native ESM default value for Browser and opaque CJS initializer for Node, both
with computed object keys, never mutable runtime JSON.parse.
JSONC alone normalizes parser-identified comments/trailing commas before strict
JSON validation; preserve original lexemes, and keep JSON/package metadata strict.
Assets export validated public-base URL strings (output-root-relative by default)
and use content-addressed byte artifacts. Relative CSS fetches resolve from the
emitted stylesheet, never by applying the public base twice. Bases do not rename
owned paths or native requests. Prepare resource wrappers before memory-hit
checks; generated text keys parse reuse and normalized bases key output reuse.
file_bytes includes text and resources; publication ownership/hash/
cleanup cover both. Resource paths never become executable module routes.
Stylesheet routes remain input-root → owned output-relative .css paths under
custom suffixes, public bases and aliases; aliases forward the same native job.
Extract CSS with parser-backed URL spans; preserve separate imports, conditions,
layers and cycles. Closure hashes include transitive original CSS/asset bytes;
recheck every input. Stylesheets maps CSS roots only; caller-loaded styles remain
application-ordered. Never flatten CSS imports or mount their discovered
dependencies as unconditional JS effects.
Validate css_modules before graph loading; central resource selection precedes
assets for entries/imports/contexts. Custom suffixes identify CSS; Everything
selects known CSS inputs, never arbitrary JS/JSON/binary graph inputs. CSS imports
use their own selection, not an inherited module flag.
Keep CSS Oxc pinned separately from JS Oxc and use its re-exported allocator
(locked 0.137.0; Rust >=1.95). Own decoded selector names, original checked UTF-8
spans, scope markers and composition requests; never persist its arena nodes.
Normalize pinned CSS decoder NULs to U+FFFD at owned identifier/string boundaries.
Recovery/raw selector syntax is an explicit error, not partially scoped success.
Selected modules default classes/IDs to local; bare scopes last to the selector
list boundary, functional scopes restore their caller's scope. Selector-bearing
pseudos and @supports selector() recurse through Oxc. Reparse only Oxc's raw
functional scope arguments as complete synthetic rules, rejecting recovery,
extra statements and incomplete consumption; translate every owned span back.
Expand functional scope lists at their owning complex selector, never through
:is() (which changes specificity). Preserve comment trivia without introducing
descendant whitespace; cap expansions and reject ambiguous type-selector joins.
Export locals only; fragment URLs rewrite only known local IDs, not classes or
global/unknown/external fragments. Names in attributes/strings/comments stay data.
Collect module composes independently of selector edits; resolve all local forward
references and explicit global tokens before generating loader mappings. Keep
own scoped tokens first and declaration order with deterministic deduplication.
Only single local class rule targets are legal; reject nested/multi-target/global
targets, unknown locals and cycles with original paths/spans, even when unused.
Remove declarations through checked original-byte edits; global CSS stays intact.
Mapping pruning never prunes stylesheet rules. Discover cross-file composition
closure through the bounded import resolver/inventory before preparing stubs;
require exported original local names, not camelCase aliases or global-only names.
Solve file composition dependencies before consumers; reject file composition
cycles independently of supported CSS @import cycles. Only actual JS/composition
evaluation dependencies get native static stub imports, in declaration order;
render-only CSS discovery never makes conditional @imports unconditional mounts.
Hash resolved composition identities and their complete CSS/resource closures;
recheck all originals and invalidate consumer/reverse users on closure edits,
including unchanged caller-loaded stub bytes. Persist no resolved CSS graph state.
Scope identities hash css-local-v1 plus normalized
project-relative path and decoded symbol into _ plus ten base64url characters;
class/ID identities coincide. Reject detected hash and camelCase export collisions.
Animation identifier definitions/references use Oxc keyframe/declaration/value
facts and a separate css-animation-v1 identity domain, never JS exports.
Resolve shorthand slots per comma group before interpreting a name; preserve
strings, custom properties, timing functions and opaque runtime groups.
Treat auto as a name candidate; quote explicit global keyword references and
preserve token boundaries when removing animation scope wrappers.
Explicit local(name)/global(name), optionally colon-prefixed, require one ident;
reject nested/ambiguous module-only values. Reparse colon escapes only through
same-length Oxc parse views and retain original-byte checked edits.
ICSS :import/:export and @value remain unsupported.
Keep each mapping a pure native ESM binding with safe private names/string aliases,
never a default object. Export aliases are nonexecuting connectors even while
prepared DOM operations stay conservative. Global CSS continues using cssparser
URL tokenization; URL/selector edits share one checked original-byte pipeline.
Closure resource names include transform identity for selected stylesheets,
including transitive CSS imports; loaders never affect scoped name identity.
Hash transform implementation and CSS parser identity into resource names,
independently of loader-only options. Configuration-only transitions retain
unchanged JS parse facts; low-level emission consumes generated text, never CSS.
CSS loading strategies consume explicit stylesheet/public-URL/mapping inputs,
never parse CSS or choose names. CallerLoaded is the default; AutoMount is opt-in
native ESM with a URL-string default and synchronous link insertion, never a
readiness promise or DOM-global deduplication. Missing document explicitly throws.
Keep prepared CSS DOM writes/throws conservative under Ecco, even with unused
exports or caller-supplied pure DOM contracts; activation still follows native
static/dynamic/deferred-require timing.
Carry prepared text and parser path/format together into cache loading.
CSS emission accepts only sealed plain-JS modes matching module facts; never
infer a wrapper mode from `.css` or bypass source/record provenance checks.
Output fingerprints independently include discovered stylesheet closure identities,
even when a declaration-only edit leaves caller-loaded mapping text unchanged.
Caller-loaded stubs have no relocations. AutoMount carries generated construction
operation spans into typed build-root URL assembly parts; never scan placeholders.
Resolve relative public bases from the generation root using the actual final
owner's import.meta.url after pruning/renaming, never the source/document/alias.
Root-relative bases use that origin and CDN bases stay absolute. Relocation
destinations remain derived, never persisted in source sidecars.

Rewrites use checked original UTF-8 byte spans. Debug-validate final executable
artifacts once after naming; release never parses output. Keep identifier,
provenance, target and edit checks always on; strip hints through cached operations.
Never apply overlapping recipe edits independently. Keep native ESM
live bindings/TDZ; no snapshot alias exports. No TS/JSX lowering or source maps.
Instantiate cached operation parents/source slots with original binding/request
IDs; replacing parents own nested rewrites, never reconnect generated text.
Materialize complete recipe MultiSpans, not just segment ownership ranges.
Split declarators share source prefixes, not binding/effect ownership; terminate
each reconstructed declaration independently.
Selected deferred reconstruction shares native recipes/name gates: keep functions
and var-lowered bindings at ESM root, defer assignments/effects in source order,
cache success/all thrown values separately, and reject reentry. Only this checked
synchronous acyclic mode waives TDZ; ordinary native emission remains unchanged.
Internal esm_plan_activated accepts explicit certified placements; ordinary
emission derives single-parent static-island placements from all retained uses,
public/eager constraints and cached AST namespace observations. Never infer
namespace safety from reachability alone. Escapes/reflection stay native;
receiver calls require unreassigned functions and no island `this` observations.
Deferred closures keep one root-binding guard per
source, with source-ordered static activation inside the guard. Dynamic recipe
operations yield before local activation or load the final typed owner then
invoke its exported guard. Per-island getter literals support certified live
reads/receivers/identity only; reflective/escaped namespaces remain native.
Availability owners are not independently executable source namespace routes.
Small libraries use output.min_chunk_bytes (zero disables), retained recipe-piece
byte sums and exact node_roots masks. Greedy path/segment seeds rank against the
current union by Jaccard overlap; never merge disjoint or individually large
islands. Physical ownership never replaces logical reachability. Effectful
members reuse checked source guards; guard their synchronous closure without
stealing existing parent/static owners. Pure binding merges must not introduce
evaluation imports or import cycles. Keep requests typed through final naming.
Generated async extraction is output.synthesize_async_boundaries opt-in only.
Derive checked reference-occurrence → segment targets before placement; remove
replaced binding demands and function-to-source evaluation demands, recomputing
exact masks without moving unrelated effects. Initial islands are private
capture-free function declarations in dependency-free, non-TLA ESM sources;
all retained incoming uses must admit suspension. Internal recursion stays sync.
Use cached reference operations and typed final-owner requests, never synthetic
DependencyIds, source-cache policy or rendered-text decisions. Plain calls must
not acquire a namespace receiver; shorthand keys survive. Co-located functions
still await; declaration-only islands need no initialization guard.

The symbol linker emits scope-hoisted direct ESM bindings; the rejected
registration/cell/alias/subscriber/generator-dispatch/capture-cycle backend is
deleted. Use native imports/exports across actual output chunks, not per-source
facade families. Public entry/export boundaries constrain output; root sets are
reachability analysis, not unconditional physical partitions or module seams.
Artifact ownership names actual executable files; merged private sources have
no independently executable namespace path in `Artifacts::modules`.
Lower original static imports/references to typed semantic obligations: binding
availability, mandatory execution/effects, activation/deferred/TLA timing, exact
namespace/public surfaces and external native validation. Preserve those
obligations, not literal source import topology. Solve physical segment owners
before reconstructing same-owner locals and cross-owner internal imports/exports.
Source-private visibility and missing export addresses never impose co-location;
real execution/binding compatibility still can. Internal chunk exports may expose
private bindings, but public namespace projections must never leak them. Use native
public projection wrappers only for actual observable surfaces, not facade families.
This supersedes the private-declaration/deferred-reader co-location rule.
Keep deferred binding edges abstract, never eager execution; scheduling edges
must not retain segments. Plan routes/projections explicitly, not during emission.
`native_connections::Connections` is immutable post-placement derived state:
owner-local names, canonical cut-edge addresses, typed evaluation/import/export
requests and exact namespace routes. Placement checks the same owner-local
declaration/import name plan consumed by connections and emission; keep native
read-only imports and inference-sensitive source names. Export only demanded
private cut bindings.
Split a public projection from its body owner only when internal transport adds
names; static/dynamic namespaces and require-of-ESM all use the public route,
lexical links use body owners. Module members still name evaluation bodies.
Validate reconstructed typed static connections, including projections, against
source initialization phases after transparent empty-job pruning and runtime
insertion; validate final targets and root routes against actual artifacts. Opaque CJS
activation retains its separate native contract; unknown external surfaces keep
original native linking clauses, not invented canonical exports.
Never greedily absorb whole modules or use SCC completeness as relocation proof.
Mandatory execution components and consumer demand choose segment owners.
Emit a complete exactly-once SegmentKey schedule per owner, using full cached
MultiSpan recipes, never module DFS body discovery. Check transitive physical
import activation against each entrance's lexical-initialization/phase projection;
each native body event must see the same initialized lexical bindings, including
abrupt completion and escaped namespaces. Pure events commute only between phases.
Index owner references once per ownership revision and cache scalar activation
summaries per owner, never transitive symbol sets or per-candidate owner scans.
Pure prefixes before the first abrupt phase can use shared binding jobs only
with explicit original-source imports after its static requests and before its
body and the initialization projection proof. Preserve mandatory prefix-to-phase
edges through those dependencies. Retain original prefix SegmentKeys when
rescheduling after owner remapping; owner-level dependencies cannot recover them.
Never apply prefix movement to SCCs, async closures, forward refs or opaque inputs.
The single-entrance fast path is bounded to a complete acyclic synchronous closure
and checks its explicit source-ordered segment schedule against mandatory edges.
Keep initialization/await phases and observable namespace/name/read-only
boundaries native, not cells. Logical artifact projections
may overlap or contain several body owners; root sets do not prescribe files.
Native dynamic imports rewrite only literal specifiers to checked occurrence routes.
Protect complete namespaces for full observations; selected projections retain
independent export roots and original-target evaluation. Keep statically async source modules as
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
Reduce body-free connector SCCs on the reconstructed typed artifact graph, not
source topology. Preserve each entrance's ordered outgoing obligations; retain
connectors in SCCs containing or reaching a nonempty or native-identity job,
including shared descendant-failure caching across entrances.
Keep cyclic connector evaluation barriers: revisiting an evaluating job must not
execute its remaining dependencies early. Hoisted availability cannot introduce
new transitive native-initializer activation across a source phase.
Use cached placement/name constraints, not trial emission. Reduce connector flags
over retained segments only; namespace, SCC and activation guards stay derived.
Prepare immutable occupied/default-name facts once per linked emission, shared
through whole-tree attempts, placement and chunk emission. Keep owner routing
and allocated-name overlays separate; never clone global name sets per owner.
Export-only names constrain default allocation, not local import aliases.
Sort canonical definitions once by source path/binding offset. Partition that
order by actual segment ownership after each placement revision; reuse the final
owner slices for local naming and private exports, never source-file seams.
ExecutionOptions bounds sequential preparation/output-owner stages; one is inline.
The coordinator deduplicates canonical paths and replenishes script jobs after
each completion; outstanding reads/results together never exceed the worker bound.
Workers read/check caches/extract owned facts with private arenas and shared
PreparedDeclarations. Pending frontiers contain paths, never source buffers.
Resolution and graph mutation stay serial. Resource/CSS processing uses a canonical
serial frontier after scripts drain; do not serially pre-read ordinary scripts.
Select source errors by canonical path, sort cache warnings and stop resource
processing at its first serial failure. Drain/join workers on spawn/panic errors.
Keep retention/placement/effects/global names serial, sharing checked
Sync graph/facts and prepared canonical order without copies. Settle cross-owner
addresses before dispatch; each ESM/CJS owner has private renderer/name/routing
state. Merge code/errors in canonical owner order, never completion order.
Workers return typed requests, never publish/prune/render; finalization stays
serial. Aggregate real worker source/rewrite/validation/declaration-hash counters.
Execution counts do not invalidate output. Preserve final source/inventory checks
and last-successful product identity on failure.
Tracing stays subscriber-free in libraries. Propagate caller Dispatch and parent
Span into both scoped worker stages; enter job spans only after dequeue, never
across queue waits. Record numeric IDs/counts, not paths, source/options or payloads.
The example's opt-in local trace must flush on returned errors without changing
stdout JSON; create new files outside inventory/output, never overwrite inputs.
Keep pending invalidations across failed attempts until the next successful
product/no-op, even when successfully prepared source facts were cached meanwhile.
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
The example driver accepts only bounded explicit fixture output/CSS/resolveConditions/workers config,
never general BuildOptions or implicit project config; keep positional calls.
resolveConditions extends Node defaults; the fixture observer explicitly passes
each condition to Node via --conditions, never assumes Node reads our config.
CSS fixtures syntax-check source JS, then run a separate native output observer;
raw Node cannot import CSS. Keep ordinary JS source/output differential checks,
owned resource inventory validation and input/build/backend/output stages intact.
DOM recording proves insertion/activation, never browser rendering or readiness.
Canonical symbols use owned (file, BindingId) identities and checked export routes,
including barrels/redeclarations. Allocated lexical names are not observable source
names or rewrite instructions; boundary values retain typed graph identities.
No hooks, code JSON, source loading/assembly, eval or runtime compilation.
Keep native live bindings, TDZ and const semantics. Outside explicit generated
boundary opt-in, only source await/for-await events suspend evaluation; never
wrap every module in async.
Derive async status over static runtime dependencies and reject all requires of
async ESM with original require/TLA spans, including deferred sites.
Do not persist derived async status. Never run a not-yet-evaluated module from a
namespace facade; late completion may only finish newly available binding parts
of an already evaluated module. Explicit externals remain typed native requests,
never artifact paths. Keep source-module seams and conservative inferred effects
when externals are present; unknown star surfaces belong to native linking.
Preserve static request order across internal and external dependencies.
Manifest external nodes have no fabricated local artifact owners.
Each CJS input is one opaque segment bridged by a logical ESM facade; both
participate in ordinary usage analysis/linking/grouping, never a standalone
native .cjs-per-input fallback. Preserve early cache allocation, wrapper
this/exports aliases/current exports and retry after throws.
Initialize factory-local __filename/__dirname parameters from actual output
availability paths; retain var/function redeclarations and nested shadows.
When CJS or a context is retained, add target-appropriate intrinsics as the first
dependency of every emitted job after empty-job pruning. Capture shim intrinsics once before application
code; include the support file in chunk projections and use it in CJS aliases.
Never rewrite application built-in lookups or activate factories from the helper.
Chunk helper identifiers must avoid all
source bindings and free references, including in opaque factories.
ESM environment probes remain native free reads; name allocation must not capture
module/exports/require with merged lexical bindings or helpers. Never inject CJS
globals, fold guards to false, or infer format from those probes.
Preserve unresolved probe read sites during Ecco refinement, independently of
graph-wide TDZ barriers; default global purity cannot erase getters/errors.
Public builds add missing probe read-effect contracts before preparing/fingerprinting
declarations; preserve explicit user contracts. Propagate through inferred calls.
Low-level undeclared deferred probes disable refinement, never silently infer purity.
Pure CJS factories group by availability roots, with one lexical cache per
initializer, never a global ESM table. Facade parts group only at equal consumer
evaluation seams; adjacent sorted snapshots may share a native job. Namespace
bridges keep their canonical default-export seam. Public Node CJS entries alone
need real CJS wrappers; internal sources have no fake independent module path.
Reverse checked requests supply original names; default is full exports,
__esModule never unwraps, names snapshot once. Namespace keys snapshot once
and reads forward to captured exports. Node require-of-ESM uses native evaluation
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

Lazy certification is disabled unless OutputOptions.lazy_runtimes explicitly
registers exact module/export constructor identities and authorizes split-specific
warming plus removable/synthesizable module allocation. Resolve immutable aliases
and reexports, never wrapper spelling hints. Certify every module-binding reference
before retention; any escape/unknown wrapper/direct operation rejects the instance.
Initializer-free literal options require checked cached structural evidence.
Certification preserves original slots/effect/capture provenance, library wrappers,
governance/selection caches and action arguments. Apply checked overlays before
retention: consume module allocations/broad imports and selector module references,
never mutate cached facts. Invocation-only removal preserves residual argument
effects; PreservedSlot edges evaluate/capture options at the original site without
demanding the removed module binding. Symbol definitions and execution accesses
also exclude that binding; only proven invocation barriers are consumed.
Keep constructor/wrapper/getter dependencies.
Only live selector values activate independent selected-export roots, carrying
original-barrel evaluation, to a nested/dead fixed point. Lazy activation origins
identify declaration/candidate/selector; their source_edge is original-import
provenance, not a fabricated or retained dependency. Stamp graph revision, effect policy and canonical runtime
configuration; configuration-only edits reuse source extraction.
Registered allocation values are not unconditional ModuleEffect demands.
Keep residual argument/forced effects and original initialization/throw phases;
compare transformed per-entrance binding projections during native placement.
Share effective reference identities through naming/activation/placement checks,
including generated module-constructor reads at selectors, never removed bindings.
`link::native_placement` exposes immutable graph/plan-local owners, schedules,
cut bindings and LazyWiring inputs for later Connections synthesis. Options name
one original PreservedOptionsValue owner/span, never copied expressions.
Connections groups lazy wiring by final source owner/original module/destination
projection and synthesizes only real registered classes plus selected native ESM
projections with original barrel evaluation. Owner-local instance bindings initialize
at original selector module-argument slots, never eager wrapper execution.
Read constructors through collision-safe owner-scope readers at allocation time;
same-file nested parameters must never capture injected constructor reads.
Options initialize once through their original cached slot/phase and cross owners
through real read-only native imports; exclude transport names from public surfaces.
Elided wrappers consume module/getter reads but retain genuine extra arguments.
Lazy activation origins never populate source-import occurrence rewrite maps.
Selectors inside their own module initializer and external-native clause mode
remain whole-instance fallbacks. Default low-level emission has no registrations;
configured plans require esm_with_options. Deferred islands decline lazy plans.

`partition` and public builds use precise planning; `partition_conservative`
is the explicit namespace reference. Per-requested-name `AsyncExport` roots
share an `AsyncSiteId` activation, never union symbols into an occurrence root.
Each seeds original-target evaluation plus its checked route; pure connectors
must not pull sibling specifiers. Namespace-binding provenance preserves checked
named CJS snapshot requests even when local export routes skip import declarations;
evaluation alone never demands those snapshots.
`async_activations` and manifest `demand_roots`
project one occurrence onto independent masks while retaining the original target.
Typed post-placement Connections project each selected activation onto native
live reexports after original-target evaluation. Route unchanged and reconstructed
imports by occurrence, never target. Selected pure owners keep exact root masks;
small-library certification must reject any complete initializer closure containing
a selected activation target, not only islands with selected masks.
Full/public consumers keep complete
namespaces; evaluation-only missing properties never invent exported bindings.
Root identities use entry IDs or async source occurrences/demands; context candidates
extend that occurrence identity with their target, never replace the source site.
Capture relative patterns against the bounded inventory and invalidate on
candidate additions/removals. Expand checked edges without inventing source IDs.
Compose original arguments through cached operations; evaluate once at the source
site, preserving lazy activation, native rejection timing and CJS cached identity.
Canonical diagnostic nodes use paths, not insertion-dependent file numbers.
Root traversal follows synchronous retention constraints only; namespace
binding demand differs from module evaluation. Empty modules keep identities.
Partition only reachable async sites to a fixed point; dead nodes have empty
root sets. Root-set equivalence is analysis, not a prohibition on scope hoisting.
Manifest chunk edges/load sets describe availability, never evaluation order.
Preserve evaluation identity without mandating per-source facade families.
Async links never become sync edges.
