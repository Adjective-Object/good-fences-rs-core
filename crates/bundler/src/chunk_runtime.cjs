'use strict';
// Registrations contain syntax, never source strings. Loading code is not evaluation.
const globalObject = globalThis;
const BindingReferenceError = ReferenceError;
const BindingTypeError = TypeError;
const NativeMap = Map;
const mapGet = Map.prototype.get;
const mapSet = Map.prototype.set;
const mapHas = Map.prototype.has;
const mapDelete = Map.prototype.delete;
class InternalMap extends NativeMap {
  get(key) { return mapGet.call(this, key); }
  set(key, value) { return mapSet.call(this, key, value); }
  has(key) { return mapHas.call(this, key); }
  delete(key) { return mapDelete.call(this, key); }
}
const modules = new InternalMap();
const cells = new InternalMap();
const segments = new InternalMap();
const proxies = new InternalMap();
const pendingProxies = new InternalMap();
const missing = Symbol('uninitialized');
let observing;

function cell(key, name, kind) {
  let result = cells.get(key);
  if (result) {
    if (kind) result.configure(name, kind);
    return result;
  }
  let value = kind === 'var' ? undefined : missing;
  const listeners = new Set();
  const read = () => {
    if (observing) listeners.add(observing);
    if (value === missing) throw new BindingReferenceError(`Cannot access '${name}' before initialization`);
    return value;
  };
  const put = next => {
    value = next;
    for (const listener of listeners) listener(next);
  };
  result = {
    get declared() { return !!kind; },
    configure(nextName, nextKind) {
      if (kind) return;
      name = nextName;
      kind = nextKind;
      if (kind === 'var' && value === missing) put(undefined);
    },
    get v() { return read(); },
    set v(next) {
      read();
      if (kind === 'const' || kind === 'import') throw new BindingTypeError('Assignment to constant variable.');
      put(next);
    },
    set i(next) { put(next); },
    watch(listener) {
      listeners.add(listener);
      if (value !== missing) listener(value);
    }
  };
  cells.set(key, result);
  return result;
}

function moduleRecord(id, commonjs) {
  modules.set(id, {commonjs, state: 0, error: undefined});
}

function alias(key, get) {
  cells.set(key, {
    get v() { return get(); },
    set v(_) { throw new BindingTypeError('Assignment to constant variable.'); },
    watch(listener) { listener(get()); }
  });
}

function watch(get, listener) {
  const update = () => {
    const previous = observing;
    observing = update;
    try { listener(get()); }
    catch (error) { if (!(error instanceof BindingReferenceError)) throw error; }
    finally { observing = previous; }
  };
  update();
}

function initial(key) {
  const binding = cell(key);
  return binding.declared ? binding.v : undefined;
}

function globalCell(name) {
  return {
    get v() {
      if (!(name in globalObject)) throw new BindingReferenceError(`${name} is not defined`);
      return globalObject[name];
    },
    set v(value) {
      if (!(name in globalObject)) throw new BindingReferenceError(`${name} is not defined`);
      globalObject[name] = value;
    }
  };
}

function typeofGlobal(name) {
  return name in globalObject ? typeof globalObject[name] : 'undefined';
}

function coercible(value) {
  if (value == null) throw new BindingTypeError('Cannot destructure null or undefined');
  return value;
}

function propertyKey(value) {
  return Reflect.ownKeys({[value]: 0})[0];
}

function rest(value, excluded) {
  const result = {};
  for (const key of Reflect.ownKeys(Object(value))) {
    if (excluded.includes(key)) continue;
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor && descriptor.enumerable) {
      Object.defineProperty(result, key, {value: value[key], enumerable: true, writable: true, configurable: true});
    }
  }
  return result;
}

function iterator(value) {
  const result = value[Symbol.iterator]();
  if (Object(result) !== result) throw new BindingTypeError('Iterator is not an object');
  return {value: result, next: result.next, done: false};
}

function iteratorStep(record, read) {
  if (record.done) return undefined;
  try {
    const step = record.next.call(record.value);
    if (Object(step) !== step) throw new BindingTypeError('Iterator result is not an object');
    record.done = !!step.done;
    return record.done || !read ? undefined : step.value;
  } catch (error) {
    record.done = true;
    throw error;
  }
}

function iteratorRest(record) {
  const values = [];
  while (!record.done) {
    const value = iteratorStep(record, true);
    if (!record.done) values.push(value);
  }
  return values;
}

function iteratorClose(record, throwing) {
  if (record.done) return;
  try {
    const method = record.value.return;
    if (method != null) {
      const result = method.call(record.value);
      if (Object(result) !== result) throw new BindingTypeError('Iterator return is not an object');
    }
  } catch (error) {
    if (!throwing) throw error;
  }
}

function segment(id, index, run) {
  let list = segments.get(id);
  if (!list) segments.set(id, list = new InternalMap());
  list.set(index, {run, done: false});
}

function* execute(id) {
  const record = modules.get(id);
  if (record.state === 3) throw record.error;
  record.state = 1;
  try {
    const list = [...(segments.get(id) || [])].sort(([a], [b]) => a - b);
    for (const [, item] of list) {
      if (item.done) continue;
      yield* item.run();
      item.done = true;
    }
    record.state = 2;
  } catch (error) {
    record.state = 3;
    record.error = error;
    throw error;
  }
}

function finish(id) {
  if (modules.get(id).state !== 2) return;
  const execution = execute(id);
  let step = execution.next();
  while (!step.done) {
    if ('await' in step.value) throw new Error('Late async segment has no native evaluation job');
    step = execution.next();
  }
}

function namespace(id) {
  const record = modules.get(id);
  if (record.commonjs) return proxy(id, '*');
  return cell(`${id}:*namespace*`).v;
}

function expose(id, value) {
  cell(`${id}:*namespace*`).i = value;
}

const factories = new InternalMap();
const cache = new InternalMap();
function factory(id, run, esm) { factories.set(id, {run, esm}); }
function requireModule(id) {
  const definition = factories.get(id);
  if (definition.esm) return definition.run();
  if (cache.has(id)) return cache.get(id).exports;
  const module = {exports: {}};
  cache.set(id, module);
  try {
    definition.run.call(module.exports, module.exports, requireModule, module);
    return module.exports;
  } catch (error) {
    cache.delete(id);
    throw error;
  }
}

function proxyFactory(id, name, run) {
  let list = proxies.get(id);
  if (!list) proxies.set(id, list = new InternalMap());
  list.set(name, {run, state: 0, value: undefined});
}
function proxy(id, name) {
  const item = proxies.get(id).get(name);
  if (item.state === 2) throw item.value;
  if (!item.state) {
    try {
      item.value = item.run();
      item.state = 1;
      cell(`${id}:*proxy*:${name}`).i = item.value;
    } catch (error) {
      item.value = error;
      item.state = 2;
      throw error;
    }
  }
  return item.value;
}

function proxyAvailable(id, name) {
  if (proxies.get(id)?.has(name)) return proxy(id, name);
  pendingProxies.set(`${id}:${name}`, [id,name]);
}

function completeProxies() {
  for (const [key,[id,name]] of pendingProxies) {
    if (proxies.get(id)?.has(name)) {
      pendingProxies.delete(key);
      proxy(id,name);
    }
  }
}

function proxyValue(id, name) {
  return cell(`${id}:*proxy*:${name}`).v;
}

function asyncIterator(value) {
  const method = value[Symbol.asyncIterator];
  const asynchronous = method != null;
  const iterator = asynchronous ? method.call(value) : value[Symbol.iterator]();
  if (Object(iterator) !== iterator) throw new BindingTypeError('Iterator is not an object');
  const next = iterator.next;
  const result = {
    next: asynchronous ? function(...args) {
      return next.apply(iterator, args);
    } : async function(...args) {
      const step = iteratorResult(next.apply(iterator, args));
      return {done: step.done, value: await step.value};
    }
  };
  Object.defineProperty(result, 'return', {get() {
    const close = iterator.return;
    if (close == null) return undefined;
    return asynchronous ? function(...args) {
      return close.apply(iterator, args);
    } : async function(...args) {
      const step = iteratorResult(close.apply(iterator, args));
      return {done: step.done, value: await step.value};
    };
  }});
  return result;
}

function iteratorResult(value) {
  if (Object(value) !== value) throw new BindingTypeError('Iterator result is not an object');
  return value;
}

module.exports = {cell, alias, watch, initial, globalCell, typeofGlobal, coercible, propertyKey, rest, iterator, iteratorStep, iteratorRest, iteratorClose, moduleRecord, segment, execute, finish, namespace, expose, factory, requireModule, proxyFactory, proxy, proxyAvailable, completeProxies, proxyValue, asyncIterator, iteratorResult};
