# ecco effect declarations: Node example

Syntax implemented in `crates/ecco`; this is not a complete Node preset. These
summaries support eliminating unused module-level expressions, not moving or
duplicating retained expressions. Throwing alone does not require retention.

## Rules

Hazard slots are `property_read_effects`, `toprimitive_effects`, `in_effects`,
`instanceof_effects`, `call_effects`, `construct_effects`, and `iterate_effects`.
`in_effects` and `instanceof_effects` describe the right operand. Property reads
and membership checks include the corresponding proxy traps; hazards are flat,
not tracked per property.

- `true` means possibly effectful; `false` means proven effect-free.
- Omitted hazards default to `true`. `*_effects` sets a default; explicit slots
  override it within a declaration. `a | b` is Boolean OR in hazard expressions.
- `returns = argument` preserves identity. Otherwise, assume a fresh result,
  described by `returns.slot = expression`; omitted return hazards stay unknown.
- `returns void` means JavaScript `undefined`, not "never returns."
- `updates target.slot |= expression` adds hazards to the referenced object,
  including through aliases. Updates never clear hazards.
- `call(fn, ..., this = receiver)` describes the callee's conditional call
  effects. Unknown callees are hazardous.
- `...` accepts any arguments; `...name` also names that argument list. Neither
  performs iteration. Argument and spread evaluation are analyzed separately.

`global fn` binds an ambient global or member, excluding lexical shadows.
`export fn` binds an export of the exact enclosing module; dotted names refer
to members of exported values, not literal dotted export names.

## Node declarations

```text
global fn console.log(...) {
  call_effects = true
  returns void
}

global fn Number(value) {
  call_effects = value.toprimitive_effects
  returns.property_read_effects = false
  returns.toprimitive_effects = false
}

global fn String(value) {
  call_effects = value.toprimitive_effects
  returns.property_read_effects = false
  returns.toprimitive_effects = false
}

global fn Object.defineProperty(target, key, descriptor) {
  call_effects = true
  updates target.*_effects |= true
  returns = target
}

module "node:path" {
  export fn join(...paths) {
    call_effects = false
    returns.property_read_effects = false
    returns.toprimitive_effects = false
  }

  export fn basename(path, suffix) {
    call_effects = false
    returns.property_read_effects = false
    returns.toprimitive_effects = false
  }
}

module "node:buffer" {
  export fn Buffer.alloc(size) {
    call_effects = false
    returns.property_read_effects = false
    returns.toprimitive_effects = false
    returns.iterate_effects = false
  }

  export fn Buffer.from(...) {
    call_effects = true
    returns.property_read_effects = false
    returns.toprimitive_effects = false
    returns.iterate_effects = false
  }
}

module "node:fs" {
  export fn readFile(...) {
    call_effects = true
  }

  export fn readFileSync(...) {
    call_effects = true
  }

  export fn writeFileSync(...) {
    call_effects = true
    returns void
  }
}

module "node:process" {
  export fn cwd() {
    call_effects = false
    returns.property_read_effects = false
    returns.toprimitive_effects = false
  }

  export fn chdir(directory) {
    call_effects = true
    returns void
  }
}
```

Number/String model ordinary calls only; missing `value` is `undefined`.
Construction remains hazardous. Path functions validate strings without
coercing objects. `Buffer.alloc(size)` covers exactly one argument; unsupported
arities fall back to hazardous calls with unknown results. Buffer results assume
the standard prototype. Allocation failure alone does not require retention.
`cwd()` is discardable when unused, but cannot move across `chdir()`.

## Conditional updates

An illustrative local summary, not a Node builtin:

```text
fn installGetter(target, getter) {
  call_effects = true
  updates target.property_read_effects |= call(getter, this = target)
  returns = target
}
```

The getter effects are latent: installing it does not invoke it. Updates apply
when the summarized function is called, not when its definition is parsed.
`Object.defineProperty` above is broader because inspecting arbitrary descriptor
fields can itself invoke getters or proxies.

## Analysis requirements

The interpreter substitutes parameters and propagates updates to a fixed point
before the caller decides retention. Every consulted fact, including a currently
safe one, is a dependency; later hazards requeue its readers. Fresh object
identities must use a bounded allocation model.

These declarations assume trusted standard builtins. Replacement must invalidate
known bindings, and prototype mutations must affect dependent values. Resolving
a member's identity does not prove reading it effect-free.

Unknown and native calls may invoke callbacks, mutate or retain reachable values,
and run option getters. `call_effects = true` alone does not describe those
transitive updates. Callback/async scheduling, captures, prototype dependencies,
return overloads, and default module namespaces still need modeling; in particular,
the `readFile` sketch does not yet describe its callback.

The initial implementation matches trusted globals, exact imports, and supplied
top-level local summaries using OXC binding identities. It does not yet infer
function bodies or link source modules. Unknown execution and potentially
effectful hooks conservatively widen all value hazards; recursive call contexts
that change arguments also widen. Persistent caching and bundler integration
remain caller-side follow-up work.
