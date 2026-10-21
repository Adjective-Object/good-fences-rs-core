const record = globalThis.__quickjs_harness_record;
delete globalThis.__quickjs_harness_record;
const apply = Reflect.apply;
const define = Object.defineProperty;
const ProxyClass = Proxy;
const toPrimitive = Symbol.toPrimitive;
const hasInstance = Symbol.hasInstance;
const iterator = Symbol.iterator;
const stringify = JSON.stringify;
const isFinite = Number.isFinite;
const TypeErrorClass = TypeError;

function serializable(_key, value) {
  const type = typeof value;
  if (type === "undefined" || type === "function" || type === "symbol" ||
      type === "bigint" || (type === "number" && !isFinite(value))) {
    throw new TypeErrorClass("unsupported effect argument");
  }
  return value;
}

export function effect(name, ...args) {
  let serialized;
  try {
    if (typeof name !== "string") throw new TypeErrorClass("effect name must be a string");
    serialized = stringify(args, serializable);
  } catch (error) {
    record(typeof name === "string" ? name : "", undefined);
    throw error;
  }
  record(name, serialized);
  return args[0];
}

export function pure(value) { return value; }

export function property(label, value) {
  return {
    get value() { effect("property.get", label); return value; },
    set value(next) { effect("property.set", label, next); value = next; }
  };
}

export function coercible(label, value) {
  return {
    [toPrimitive](hint) { effect("coerce", label, hint); return value; }
  };
}

export function membership(label) {
  return new ProxyClass({}, {
    has(_target, key) { effect("has", label, key); return true; }
  });
}

export function instance(label) {
  function Instance() {}
  define(Instance, hasInstance, {
    value(value) { effect("instanceof", label, value); return true; }
  });
  return Instance;
}

export function callable(label, value) {
  return function (...args) {
    effect("call", label, this === undefined ? null : this, ...args);
    return value;
  };
}

export function constructible(label) {
  return function (...args) { effect("construct", label, ...args); };
}

export function iterable(label, values) {
  return {
    [iterator]() {
      effect("iterator", label);
      let index = 0;
      return {
        next() {
          effect("next", label, index);
          return index < values.length
            ? { value: values[index++], done: false }
            : { value: undefined, done: true };
        },
        return() { effect("return", label); return { done: true }; }
      };
    }
  };
}

export function prototype(label) {
  return new ProxyClass({}, {
    getPrototypeOf() { effect("prototype", label); return null; }
  });
}

export function globalRead(name, value) {
  define(globalThis, name, {
    configurable: true,
    get() { effect("global", name); return value; }
  });
}

export function invoke(callback, receiver, ...args) {
  return apply(callback, receiver, args);
}

export function cell(value) { return { value }; }

export function fail(label, value) {
  effect("fail", label, value);
  throw value;
}
