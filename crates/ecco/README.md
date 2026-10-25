# ecco

Effect declarations and alias-aware, parallel hazard propagation for eliminating
unused module-level JavaScript expressions. Throwing alone is not an effect.

Ecco targets statement-level bundling of massive applications; throughput,
memory use, and parallel scalability are explicit goals. Function-body inference
should produce owned contracts from inputs and referenced symbols, without
caller-specific body re-analysis or live-binding histories.

`parse_declarations` produces owned `LocalFacts`. `parse_from_ast` consumes an
existing checked OXC program and semantic model without retaining arena data;
`parse_source` is its parsing convenience entrypoint. Both return a serializable
`ParsedFile` containing lazy owned function definitions and module roots, not
evaluated hazards or specialized calls.
Persisted records expose borrowed accessors, including declaration collections
and nested templates; construct declarations with their `new` methods or the
declaration parser. Interpreter state remains separate and mutable.

`parse_from_ast_with_segments(program, semantic, facts, segments)` reuses that
same checked AST and validates unique root ownership in the caller's actual
segment ranges, including split multi-declarator declarations. Owned template
serialization is deterministic; recording never executes source function calls.

`Analysis` links those precursors through a caller-provided `Resolver` and runs
the connected analysis. `Context::from_local_facts` remains available for
constructing interpreter values and operations directly. `extract_from_ast`
and `analyze_source` retain their single-file analysis interfaces.

Evaluation uses a dependency worklist and Rayon to compute a monotone fixed
point. Updates reach all aliases, including reads preceding a mutation. Safe
facts are dependencies too. Results are only final after evaluation converges.

This is an analysis library, not a bundler integration or complete JavaScript
interpreter. Unknown calls and unsupported executable syntax conservatively
hazard values. Standard environment declarations are trusted contracts, not
runtime checks. Module resolution, persistent cache storage, and scheduling
belong to the caller.

```rust
use std::path::Path;
use ecco::{analyze_source, parse_declarations};

let declarations = parse_declarations(r#"
global fn Number(value) {
  call_effects = value.toprimitive_effects
  returns.*_effects = false
}
"#)?;
let analysis = analyze_source(
    "function convert(x) { return Number(x); } const a = 1; convert(a);",
    Path::new("input.js"),
    declarations,
)?;
let evaluation = analysis.context.evaluate();
assert!(analysis.roots.iter().all(|root| !evaluation.effect(root.effects)));
# Ok::<(), eyre::Report>(())
```

## Owned extraction and graph linking

### Borrowed FlatBuffers precursors

`ParsedFile::view()` and `views::ParsedFileView::from(wire_table)` share
allocation-free, `Copy` accessors for owned and byte-backed storage. Nested
records, instructions, binding paths and recursive contracts are returned by
value; `Sequence` reads either a slice or generated FlatBuffers vector iterator.
Strings borrow their input allocation. IDs, spans, hazards and `CallResult`
are small scalar values; no reader calls the generated object API's `unpack`.

`ParsedFile::encode_flatbuffer` writes typed Ecco tables into a caller's builder
for embedding directly in an AST module. The construction-only encoder
uses generated objects; `to_flatbuffer` is a standalone convenience.
`cache_flatbuffers::WireEcco` structurally verifies and owns standalone bytes.
`views::ModuleEcco` retains a checked `WireRecord` with nested Ecco, and other
module owners can implement `views::ParsedFileSource`.

`Analysis::add_file` accepts `Arc<dyn ParsedFileSource>` (including existing
`Arc<ParsedFile>`), validates contracts/IDs through views, and retains the owner.
Lazy runtime function templates store owner plus index, not copied instructions;
validation, resolution and replay never reconstruct a parsed file. Interpreter
value/effect maps and specialized call arguments still allocate. Declaration
contracts are explicitly copied into the existing mutable interpreter, and
definition matches into its owned result. These allocations are solve state,
not eager cache deserialization. Contract evaluation itself uses borrowed views
over shared interpreter declarations, without cloning a contract at each call.

Resolvers now receive `ExternalReferenceView` by value and match `BindingView`;
`declaration_matches` accepts either owned references or borrowed views.
Production `.seg.fb` sidecars embed these typed precursors in the AST module;
warm loading and solving retain immutable buffer owners without unpacking.
Serde remains for debug/test fixtures and the independent declaration-configuration
fingerprint, not per-file cache encoding or loading.

`ParsedFile` and `OwnedFunction` implement Serde serialization. Save only these
precursors, never `Context`, `Evaluation`, or `AnalysisResult`. Each solve
creates fresh interpreter identities and specializes calls only after linking.
Parsed files include their extraction declarations; scope-specific local
contracts remain local to the file, even when another file uses the same name.
Invalidate cached files when those declarations or extraction semantics change.
The serialized format carries a version; `add_file` validates deserialized
indices, spans, and template references before installing them.

```rust
use std::{path::Path, sync::Arc};
use ecco::{Analysis, LocalFacts, Resolution, Symbol, parse_source};
use ecco::views::{ExternalReferenceView, BindingView};

let library = Arc::new(parse_source(
    "export function read(value) { return value.key; }",
    Path::new("library.js"), LocalFacts::default(),
)?);
let read = library.symbols().iter().find(|symbol| symbol.name() == "read").unwrap().id();
let entry = Arc::new(parse_source(
    "import { read } from 'library'; read({});",
    Path::new("entry.js"), LocalFacts::default(),
)?);
let mut analysis = Analysis::new();
analysis.add_file("entry", entry)?;
analysis.add_file("library", library)?;
let result = analysis.solve(&|_: &&str, reference: ExternalReferenceView<'_>| {
    match reference.binding() {
        BindingView::Module { module, path } if module == "library" && path.iter().eq(["read"]) => {
            Ok(Resolution::Symbol(Symbol { file: "library", local: read }))
        }
        _ => Ok(Resolution::Unknown),
    }
})?;
assert!(!result.evaluation.effect(result.files["entry"].roots[0].effects));
# Ok::<(), eyre::Report>(())
```

The resolver receives each external reference's source span and global/module
binding path. Resolve through barrels to canonical `Symbol` identities, whose
local IDs are declaration starts, not OXC IDs. Anonymous/default expressions can
be addressed as `Resolution::Root` using their `RootId` and source span.
`Resolution::Declaration` selects a contract or trusted namespace prefix in the
referring file. `Resolution::Namespace` shares a module namespace identity;
static member accesses are separately resolved paths, while computed unknown
members stay conservative. Return `Unknown` only for valid unmodeled values;
missing or ambiguous exports must be errors.

All symbols and functions are registered before roots execute. Hoisted
functions are installed across all files before any root calls. Other
initializers are instantiated lazily when their bindings or root values are
demanded. Cyclic function calls share pending equations; cyclic value
initializers remain conservative. This is analysis dependency traversal, not a
module evaluation scheduler. Module initialization retention and effect-order
edges still belong to the bundler.

## Declarations

`DeclarationConfig::default().effective()` combines the independently switchable
ECMAScript 2025 and Node 24 presets with parsed `user` declaration sets. Exact
user bindings override preset bindings; duplicate user contracts and build-wide
local contracts are errors. Effective ordering is canonical for cache hashing.
`effective_for_environment(Environment::Browser)` selects `presets/browser.ecco`
instead of Node, retaining the independent `javascript` switch and user contracts.
The `node` and `browser` switches enable only their selected host; both default
to enabled, but `effective()` still selects Node. Browser contracts cover a bounded
Window environment (document/navigation/storage, fetch, timers and events), not
exhaustive DOM typings or worker environments. Host reads remain conservative,
including inside inferred functions; DOM getters, mutations, callbacks and
arbitrary APIs are not pure. Browser host-object member aliases are deliberately
not expanded into trusted namespace paths, so `globalThis.document` and unknown
`window`/`self` properties retain getter effects. Exact user read and function
contracts remain separate overrides.
The JavaScript preset covers globals, constructors/prototypes, collections,
typed arrays, Math, JSON, Reflect, Atomics, promises and iterator helpers. The Node
preset covers host globals and builtin modules, including `node:` and supported
bare aliases and their default-export member paths. Unknown or unsafe operations
remain effectful; conversions consult input hooks. Prototype methods, mutation,
callbacks and I/O mostly have conservative contracts, not specialized heap models.
Contracts assume standard environments; unknown execution still widens hazards.

`global fn console.log(...)`, `module "node:path" { export fn join(...) }`,
and `fn localName(...)` bind globals, exact module exports/members, and trusted
top-level local function declarations respectively. Fixed signatures accept
missing parameters as `undefined`; extra arguments require `...` or `...name`.
Supplied local contracts override inferred bodies.

Global accessor reads are declared separately from value hazards:

```text
global symbol ambient {
  read_effects = true
}
```

`read_effects` is a required boolean. Undeclared global reads are pure; callers
must declare every global whose getter has effects. This applies to bare reads,
`typeof`, and reads inside inferred functions, but not to shadowing lexical or
imported bindings. `read_effects = false` explicitly declares a pure read. A
symbol declaration can coexist with a `global fn` declaration of the same name:
one describes reading the binding, the other describes the callable value.
Reading an undeclared global is pure, but calling it or accessing its unknown
members remains conservative. Symbol names are single identifiers.

Function/value hazards default to true; explicit slots override `*_effects`
regardless of order. Expressions accept booleans, `argument.slot`, Boolean `|`, parentheses,
`unknown()` for unmodeled execution, and
`call(callback, argument, ...rest, this = receiver)`. Bare `...` in a call
forwards the current arguments. Named rest lists can only be spread.

`prototype_read_effects` models prototype traversal, independently of ordinary
`property_read_effects`. `instanceof` consults the left operand's prototype
hazard and the right operand's `instanceof_effects`.

Fresh results use `returns.slot = expression`. `returns = argument` preserves
identity; `returns void` gives `undefined`, while `returns unknown` makes no
identity claim. `updates argument.slot |= expression`
monotonically adds hazards; `updates argument.*_effects |= true` sets all slots.
Call expressions in hazard assignments describe latent hazards, not execution
at declaration time.

## PURE annotations

`/*#__PURE__*/` and `/*@__PURE__*/` on calls and `new` expressions override
invocation effects, including conservative unknown/unsupported bodies and
declared mutation summaries. Callee evaluation, argument evaluation, and spread
iteration still contribute effects. The annotation is retained in inferred
function summaries and honored through optional calls as well.

Known call return identities and hazards are inferred without executing updates.
Unknown returns and constructed values remain conservative: PURE describes the
invocation, not subsequent operations on its result. Annotations are trusted
caller contracts, not verified proofs.

## Function inference

OXC's semantic symbol/reference IDs connect parameters, captures, local aliases,
returns, and hazard-triggering operations. For example, `function read(x) {
return x.value; }` depends on the actual argument's `property_read_effects`;
`function identity(x) { return x; }` preserves its identity.

The extractor records owned operation summaries using the same expression
analysis as module roots. Recording does not execute bodies or apply updates.
Calls instantiate those summaries with actual arguments and receivers. Function
declarations are hoisted; function expressions, arrows, nested closures, and
callbacks retain lexical bindings. Each invocation gets independent locals,
allocations, and captured closure environments. Ordinary functions use the call
receiver; arrows capture lexical `this`. Ordinary functions expose conservative
constructor hazards; non-constructible arrows do not execute a body when
construction throws. Ordinary functions also own fresh implicit `arguments`
objects; arrows inherit the enclosing arguments binding, not a same-named global.
Arguments object members remain conservative.

Inferred effect queries subscribe to live capture bindings. Later rebinding
widens those queries, including cached declared callback queries, without
redirecting previously returned value identities.

Returns through `if`/`else`, blocks, conditionals, and logical expressions join
possible identities. Fallthrough returns `undefined`; terminal returns/throws
stop scanning that path. Both possible branches are analyzed, not proven
reachable. Continuing branches merge their lexical bindings; reading a binding
captures its current value without redirecting earlier aliases on rebinding.
Captures retained by returned closures keep exited-path alternatives when later
paths rebind the same slots.
Initializer-free `var` declarations preserve existing bindings. Supplied
contracts called from inferred bodies or through nested declared callbacks
propagate their updates normally. Hazard queries of inferred callbacks remain
latent: querying an effect does not execute its mutations. Queries involving
mutation contracts conservatively widen effects rather than simulate a
hypothetical heap.

Bare `call_effects` reads cannot specialize arguments and conservatively describe
the whole function, including declared functions. Known calls specialize the body
and separately track replacement hazards, so later updates still invalidate
earlier pure calls. Executing a potentially replaced target conservatively
widens heap and return hazards, not only that call's effect.
`SourceAnalysis.matches` and `FileAnalysis.matches` include matched contracts in
supported deferred bodies as well as module roots. Summaries are shared within the context; graph
construction is serial and fixed-point evaluation remains parallel.

## Conservative boundaries

`infer_function_contracts` analyzes top-level named functions and `const`
functions/arrows once, including exports, and returns owned `LocalFacts`;
`parse_from_ast` includes these contracts in the serializable `ParsedFile`
automatically, before any connected analysis. Dependencies are
processed before consumers, regardless of source order. Parameters, immutable
local aliases, branches, callback calls, declared updates, and return hazards
are expressed using the existing contract model. Closed contracts with fully
representable results replace body interpretation at calls. Functions requiring
linked symbols, live captures, unknown returns, or unsupported syntax keep their
owned lazy summaries, preserving connected-graph resolution, alias identities,
and PURE behavior rather than treating incomplete inference as authoritative.
Cycles, unsupported bodies, and exhausted per-function work/expansion budgets
produce unknown contracts without executing their bodies during inference.

Contract expression transfers borrow immutable declarations and local values,
separately from OXC traversal and statement/function collection. Each expression
root returns its value/effect summary, ordered deduplicated updates, consumed
work, and opacity. A transfer-local accumulator avoids copying update logs along
expression edges; collection merges outcomes in source order and carries the
remaining per-function budget. Invocation expansion borrows contracts rather
than cloning them. Common leaf hazard summaries stay compact rather than
materializing a symbolic expression for every hazard slot.

Contract inference and lazy-operation extraction lower expressions iteratively
into transient node/operand tables containing no OXC node or semantic references.
Ready transfers consume child summaries once, leaf-inward, preserving the
original grouping and operand positions. Contract visit costs retain preorder
budget boundaries, including uncharged callee-member references and PURE masking.
Lazy Context recording stays source-ordered: key/coercion/spread operations are
recorded before later operands, not postponed to their parent's transfer.
Projection storage is linear and bounded by the expression's source size (and
the contract work limit); exhausted contract planning stays unknown.
Opaque/exhausted inference skips lowering; each further projection is bounded
by the remaining function budget, not a fresh full-function allowance.
Dependency, binding and compiler expression scans also use explicit worklists.
This is sequential extraction, not a runtime scheduler or a parallel expression
pool. Statement/function nesting and declared-contract expansion retain their
existing traversal and limits; general AST/interpreter stack safety is not claimed.

`cargo bench -p ecco --bench frontier` measures inference and owned extraction
over pre-parsed small, wide and deep fixtures. It uses an explicit 64 MiB
benchmark-only stack so recursive baselines and frontier versions run identically;
production-stack regressions do not use this accommodation. Override iterations
with `ECCO_FRONTIER_ITERATIONS` (default 100, five samples).

The extractor handles simple declarations, calls, object literals, member reads,
operators, templates, conditional/logical expressions, aliases, and the function
bodies described above. Function creation remains safe even for unsupported
bodies. Default/rest/destructured parameters, async/generator functions, loops,
`try`/`catch`, and assignments conservatively widen hazards when called. Arrays,
construction, spread calls, classes, and unknown object members also remain
conservative. Imported module initialization is not included in expression roots.

Trusted namespace prefixes have ordinary member reads and share identities
between module roots and deferred bodies. Replacements, unknown
execution, and potentially effectful hooks widen all value hazards until
capture/prototype reachability is modeled. Direct `hazard` queries only consult
facts; `operation` additionally models execution of unmodeled hooks. Conditional
value joins propagate updates to every possible alias without treating intrinsic
call hazards as replacements. Recursive summary calls share exact contexts.
Declared calls widen growing argument lists; inferred calls widen changing
recursive contexts rather than grow without bound. Each explicit call site gets
its own fresh result identity.

Declared effectful global getters conservatively widen heap hazards when read.
Creating a function that reads such a global, or querying that callback's latent
effects, does not execute the getter.

Unknown-alias reachability propagates through value joins. Updates through such
aliases widen the heap only when their hazard expression becomes true, including
after later fixed-point updates. Loose equality of one known value identity with
itself does not execute coercion hooks unless opaque execution has invalidated
binding identity precision; other loose comparisons stay conservative.

Context graphs are reusable, but results are not persistently cached. Build a new
context when removing source facts or changing environment contracts: monotone
updates cannot retract hazards. The bundler owns cache/configuration integration;
Ecco provides the independent JavaScript and Node declaration presets.
