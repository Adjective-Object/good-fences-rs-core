# quickjs-bundler-harness

The [cargo-fuzz target](fuzz/README.md) compares bounded generated programs with
real native bundler output in four optimization configurations, entirely in Rust.

A native Rust QuickJS harness for effect-based bundler fuzzing. Each run receives
one example's map of module paths to JavaScript source, a selected entry, and a
fresh runtime. The only additional module is the built-in `harness` test library.
The resolver never reads modules from disk or the network.

The public runner returns `eyre::Result<HarnessRunResult>`, whose `effects:
Vec<Effect>` records operations and their arguments in execution order. Run the
source and bundled module maps separately and compare their effects exactly.
Resource exhaustion, unresolved entry evaluation, invalid modules, and uncaught
exceptions are errors, not equivalent successful empty logs.

```rust
use std::{collections::BTreeMap, path::PathBuf};
use quickjs_bundler_harness::run;

let modules = BTreeMap::from([
    (PathBuf::from("entry.js"), r#"
        import { effect } from "harness";
        import { value } from "./value.js";
        effect("answer", value, { enabled: true });
    "#.to_owned()),
    (PathBuf::from("value.js"), "export const value = 42;".to_owned()),
]);
let result = run(&modules, "entry.js")?;
assert_eq!(result.effects[0].name, "answer");
assert_eq!(result.effects[0].arguments[0], serde_json::json!(42));
# Ok::<(), eyre::Report>(())
```

The Rust API is deliberately small:

```text
run(&BTreeMap<PathBuf, String>, entry: impl AsRef<Path>) -> eyre::Result<HarnessRunResult>
run_with_limits(&BTreeMap<PathBuf, String>, entry: impl AsRef<Path>, Limits)
    -> eyre::Result<HarnessRunResult>
HarnessRunResult { effects: Vec<Effect> }
Effect { name: String, arguments: Vec<serde_json::Value> }
```

Both result types derive `Debug`, `Clone`, `PartialEq`, and `Eq`. Paths are
UTF-8, example-root-relative, normalized lexically, and matched exactly: no
extension guessing, package lookup, or filesystem canonicalization. Absolute,
URI, backslash, query/fragment, NUL, and root-escaping paths are rejected.
Only relative import specifiers and the reserved bare name `"harness"` resolve.
Duplicate normalized input paths, overriding `"harness"`, missing modules and
import attributes are errors. Static and dynamic imports use the same map and
native module cache, including cached evaluation failures. Top-level await and
promise jobs run to completion before the result is returned.

## Test-library contract

Import functions from `"harness"`. The library exercises every operation hazard
in `ecco::Hazard::ALL`, plus aliasing, callbacks, mutation, and global reads.
It is a test vocabulary for Ecco's analysis surface, not an implementation of
every ECMAScript or Node standard-library API.

The table's argument lists are the exact arguments in each resulting `Effect`.
Constructing these fixtures does not execute their deferred operations.

| Export | Trigger and recorded operation | Result |
| --- | --- | --- |
| `effect(name, ...args)` | `name`, arguments `args` | First original argument, or `undefined` |
| `pure(value)` | No effect | Original `value` |
| `property(label, value)` | `.value` read: `property.get(label)`; write: `property.set(label, next)` | Getter returns current value; setter updates it |
| `coercible(label, value)` | Conversion: `coerce(label, hint)` | Supplied primitive |
| `membership(label)` | `key in object`: `has(label, key)` | `true` |
| `instance(label)` | `value instanceof fn`: `instanceof(label, value)` | `true` |
| `callable(label, value)` | Invocation: `call(label, receiver, ...args)` | Original supplied `value`; unbound receiver is recorded as `null` |
| `constructible(label)` | `new fn(...args)`: `construct(label, ...args)` | Fresh empty object |
| `iterable(label, values)` | Iterator creation: `iterator(label)`; each `next()`: `next(label, index)`; early close: `return(label)` | Reads `values.length`/`values[index]` lazily, with zero-based index |
| `prototype(label)` | Prototype traversal: `prototype(label)` | Proxy whose reported prototype is `null` |
| `globalRead(name, value)` | Later bare/global reads: `global(name)` | Installs configurable getter; installation returns `undefined` |
| `invoke(callback, receiver, ...args)` | No extra effect; calls callback with explicit receiver and forwarded arguments | Callback result |
| `cell(value)` | No extra effect; mutable `{ value }` | Fresh cell; aliases and captured reads remain live |
| `fail(label, value)` | `fail(label, value)` | Throws original `value` after recording |

Coverage maps directly to Ecco's eight hazards: `.value` → `property_read`,
conversion → `toprimitive`, membership → `in`, custom instance check →
`instanceof`, invocation → `call`, `new` → `construct`, spreading/`for..of` →
`iterate`, and `prototype("p") instanceof Object` → `prototype_read`.
`globalRead` additionally exercises effectful global-symbol access.
`invoke` covers callback receiver/rest/forwarding; `pure`, `effect`,
`constructible`, and `globalRead` supply identity/fresh/void returns. `cell`,
ordinary closures, and property setters supply alias/mutation/capture cases.
Compare identity *in the guest* and record its boolean result, not references.

Pure controls are ordinary data properties, primitives, plain objects/functions,
arrays, and `pure`/`cell`/`invoke` without effectful callbacks. These are runtime
fixtures, not trusted Ecco declarations: no inferred purity or special analyzer
contract is promised. Input-triggered coercion, property, iterator, callback,
and serialization effects still execute normally.

## Serialization boundary

Arguments are immediately serialized to owned JSON snapshots, not retained
guest references. Use plain JSON-serializable test values: null, booleans, finite
numbers, Unicode strings, arrays, and string-keyed objects. A captured
`JSON.stringify` with a validating replacer rejects encountered `undefined`,
functions, symbols, BigInt, nonfinite numbers, sparse-array holes, and cycles;
the Rust JSON parser also rejects unpaired surrogates and excessive nesting.
Effect names must also be Unicode strings without unpaired surrogates.
Serialization/conversion errors fail the entire run, even if guest code catches them.
No unsupported argument is silently omitted or replaced with null.

Normal JSON object semantics still apply: only enumerable string keys are
visited, symbol/nonenumerable keys are ignored, `-0` becomes `0`, and prototypes,
reference identity, sharing, and Map/Set contents are not represented. Getters,
proxy traps, and `toJSON` **can run during serialization**; their effects precede
the outer snapshot's append. `toJSON` can transform a value before the replacer
sees it. Keep fuzz observations plain, or observe such objects explicitly, e.g.
`effect("value", object.value)`. Do not pass the fixture functions, symbols,
cyclic receivers, or an undefined callback argument as effect arguments.
The function return remains the original guest value, never its JSON copy.

## Containment and fuzzing

Use native QuickJS without QuickJS libc host bindings (`std`/`os`), filesystem,
network, process, or native-module bindings. Relative imports resolve to the supplied map;
unknown imports fail. Dynamic imports and pending promise jobs use that same
resolver. Each call has bounded memory, stack, execution, jobs, and effect output.

`Limits::default()` allows 32 MiB QuickJS heap, 512 KiB QuickJS stack, 10,000
interrupt polls, 10,000 pending jobs, 10,000 effects, 100,000 recorded JSON values
(including nested values), 8 MiB total serialized output/name bytes, 8 MiB total
input source/path bytes, and 1,024 input modules. `run_with_limits` adjusts these
budgets; memory and stack limits must be nonzero. Execution and recorder
exhaustion cannot be turned into successful partial logs by catching an error.
The runner drains all queued jobs, rejects unhandled promise rejections
(including `undefined`), then checks entry settlement. An entry still awaiting
an unresolvable promise is an explicit error. An unrelated, unawaited pending
promise with no jobs does not prevent success.

Interrupt polls are deterministic engine instruction checkpoints, not an exact
instruction count or a wall-clock deadline. Memory limits cover the engine heap,
not total process RSS; owned source/results are separately bounded. The engine
can execute costly native builtins between interrupt polls. Engine memory/stack
exceptions follow JavaScript exception handling; uncaught ones fail the run.
There is no virtual clock, random seeding, timer service, or nondeterminism normalization:
avoid `Date`/`Math.random` when comparing exact logs.

This is engine-level containment, not an operating-system security sandbox.
Native QuickJS bugs can still crash or compromise the calling fuzz process.
Run fuzz workers in separate processes or an OS sandbox when testing hostile
input; the library must not claim otherwise.

TDZ-error preservation is not a bundler requirement. Keep TDZ-specific probes
separate from ordinary exact effect-log comparisons. Added async suspension is
permitted only when explicitly enabled in the bundler; checkpoint deterministic
effects rather than relying on races with outside work.
