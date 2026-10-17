import { registerHooks, createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const parse = JSON.parse, hasOwn = Object.hasOwn, Url = URL, LoadError = Error;
const read = readFileSync;
const ownNames = Object.getOwnPropertyNames, define = Object.defineProperty;
const create = Object.create, freeze = Object.freeze, box = Object;
const moduleTag = Symbol.toStringTag;
const get = Function.call.bind(Map.prototype.get);
const has = Function.call.bind(Map.prototype.has);
const set = Function.call.bind(Map.prototype.set);
const modules = new Map(Object.entries(config.modules));
const urls = new Map([...modules.keys()].map(token => [
  token, new Url(`native/${token.slice('good-fences:'.length)}.${config.formats[token] === 'commonjs' ? 'cjs' : 'mjs'}`, import.meta.url).href,
]));
const sources = new Map([...modules].map(([token, pieces]) => [get(urls, token), pieces]));
const formats = new Map([...modules.keys()].map(token => [get(urls, token), config.formats[token]]));
const payloads = new Map();
const require = createRequire(import.meta.url);

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (has(sources, specifier)) return { url: specifier, shortCircuit: true };
    if (has(sources, context.parentURL) && specifier.startsWith('./')) {
      const url = new Url(specifier, context.parentURL).href;
      if (has(sources, url)) return { url, shortCircuit: true };
    }
    if (has(urls, specifier) && (has(sources, context.parentURL) || context.parentURL === import.meta.url)) {
      return { url: get(urls, specifier), shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (!has(sources, url)) return nextLoad(url, context);
    const pieces = get(sources, url);
    let source = '';
    for (let i = 0; i < pieces.length; i++) {
      const filename = pieces[i][0], key = pieces[i][1];
      if (!has(payloads, filename)) {
        set(payloads, filename, parse(read(new Url(filename, import.meta.url), 'utf8')));
      }
      const payload = get(payloads, filename);
      if (!hasOwn(payload, key) || typeof payload[key] !== 'string') {
        throw new LoadError(`Missing module chunk piece: ${filename} ${key}`);
      }
      source += payload[key];
    }
    return { format: get(formats, url), source, shortCircuit: true };
  },
});

export async function loadEntry(name) {
  if (!hasOwn(config.entries, name)) throw new LoadError(`Unknown bundle entry: ${name}`);
  const result = [];
  const tokens = config.entries[name];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    // CJS entries start in the CJS loader, so require->ESM->CJS cycles see
    // an allocated CJS record rather than an evaluating ESM entry proxy.
    const cjs = token.endsWith(':namespace');
    if (cjs) requireModule(token.slice(0, -':namespace'.length));
    const namespace = await import(get(urls, token));
    result[result.length] = cjs ? namespace.default : namespace;
  }
  return result.length === 1 ? result[0] : result;
}

export function requireModule(token) {
  if (!has(urls, token) || config.formats[token] !== 'commonjs') {
    throw new LoadError(`Unknown bundle CommonJS module: ${token}`);
  }
  return require(token);
}

export function commonjsNamespace(value) {
  const namespace = create(null);
  define(namespace, 'default', { enumerable: true, value });
  if (value != null) {
    const target = box(value), keys = ownNames(target);
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (key !== 'default') define(namespace, key, { enumerable: true, get: () => target[key] });
    }
  }
  define(namespace, moduleTag, { value: 'Module' });
  return freeze(namespace);
}
