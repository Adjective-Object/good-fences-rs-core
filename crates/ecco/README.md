# ecco

Effect declarations and alias-aware, parallel hazard propagation for eliminating
unused module-level JavaScript expressions. Throwing alone is not an effect.

`parse_declarations` produces owned `LocalFacts`. `Context::from_local_facts`
builds an interpreter context; callers can construct values and operations
directly or use `extract_from_ast` with an existing OXC program and semantic
model. `analyze_source` is the parsing convenience entrypoint.

Evaluation uses a dependency worklist and Rayon to compute a monotone fixed
point. Updates reach all aliases, including reads preceding a mutation. Safe
facts are dependencies too. Results are only final after evaluation converges.

This is an analysis library, not a bundler integration or complete JavaScript
interpreter. Unknown calls and unsupported executable syntax conservatively
hazard values. Standard environment declarations are trusted contracts, not
runtime checks. Cross-module linking, persistent caching, and scheduling belong
to the caller.

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

## Declarations

`global fn console.log(...)`, `module "node:path" { export fn join(...) }`,
and `fn localName(...)` bind globals, exact module exports/members, and trusted
top-level local function declarations respectively. Fixed signatures accept
missing parameters as `undefined`; extra arguments require `...` or `...name`.
Supplied local contracts override inferred bodies.

Hazards default to true; explicit slots override `*_effects` regardless of
order. Expressions accept booleans, `argument.slot`, Boolean `|`, parentheses,
and `call(callback, argument, ...rest, this = receiver)`. Bare `...` in a call
forwards the current arguments. Named rest lists can only be spread.

Fresh results use `returns.slot = expression`. `returns = argument` preserves
identity; `returns void` gives `undefined`. `updates argument.slot |= expression`
monotonically adds hazards; `updates argument.*_effects |= true` sets all slots.
Call expressions in hazard assignments describe latent hazards, not execution
at declaration time.

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
receiver; arrows capture lexical `this`.

Returns through `if`/`else`, blocks, conditionals, and logical expressions join
possible identities. Fallthrough returns `undefined`; terminal returns/throws
stop scanning that path. Both possible branches are analyzed, not proven
reachable. Supplied contracts called from inferred bodies propagate their
updates normally. Hazard queries of inferred callbacks remain latent: querying
an effect does not execute its mutations. Queries involving mutation contracts
conservatively widen effects rather than simulate a hypothetical heap.

Bare `call_effects` reads cannot specialize arguments and conservatively describe
the whole function. Known calls specialize the body and separately track
replacement hazards, so later updates still invalidate earlier pure calls.
`SourceAnalysis.matches` includes matched contracts in supported deferred bodies
as well as module roots. Summaries are shared within the context; graph
construction is serial and fixed-point evaluation remains parallel.

## Conservative boundaries

The extractor handles simple declarations, calls, object literals, member reads,
operators, templates, conditional/logical expressions, aliases, and the function
bodies described above. Function creation remains safe even for unsupported
bodies. Default/rest/destructured parameters, async/generator functions, loops,
`try`/`catch`, and assignments conservatively widen hazards when called. Arrays,
construction, spread calls, classes, and unknown object members also remain
conservative. Imported module initialization is not included in expression roots.

Trusted namespace prefixes have ordinary member reads. Replacements, unknown
execution, and potentially effectful hooks widen all value hazards until
capture/prototype reachability is modeled. Direct `hazard` queries only consult
facts; `operation` additionally models execution of unmodeled hooks. Conditional
value joins propagate updates to every possible alias. Recursive summary calls
share exact contexts; changing recursive contexts widen rather than grow without
bound. Each explicit call site gets its own fresh result identity.

Context graphs are reusable, but results are not persistently cached. Build a new
context when removing source facts or changing environment contracts: monotone
updates cannot retract hazards. Cross-module reference linking and a complete
Node/browser preset are not implemented.
