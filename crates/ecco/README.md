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
let analysis = analyze_source("const a = 1; Number(a);", Path::new("input.js"), declarations)?;
let evaluation = analysis.context.evaluate();
assert!(analysis.roots.iter().all(|root| !evaluation.effect(root.effects)));
# Ok::<(), eyre::Report>(())
```

## Declarations

`global fn console.log(...)`, `module "node:path" { export fn join(...) }`,
and `fn localName(...)` bind globals, exact module exports/members, and trusted
top-level local function declarations respectively. Fixed signatures accept
missing parameters as `undefined`; extra arguments require `...` or `...name`.
Local summaries are supplied contracts, not inferred function bodies.

Hazards default to true; explicit slots override `*_effects` regardless of
order. Expressions accept booleans, `argument.slot`, Boolean `|`, parentheses,
and `call(callback, argument, ...rest, this = receiver)`. Bare `...` in a call
forwards the current arguments. Named rest lists can only be spread.

Fresh results use `returns.slot = expression`. `returns = argument` preserves
identity; `returns void` gives `undefined`. `updates argument.slot |= expression`
monotonically adds hazards; `updates argument.*_effects |= true` sets all slots.
Call expressions in hazard assignments describe latent hazards, not execution
at declaration time.

## Conservative boundaries

The extractor handles simple declarations, calls, object literals, member reads,
operators, templates, conditional/logical expressions, and aliases. Function
creation is safe, but bodies are not inferred yet. Arrays, construction, spread
calls, destructuring, arbitrary control flow, classes, and assignment currently
fall back conservatively. Imported module initialization is not included in the
returned expression roots.

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
