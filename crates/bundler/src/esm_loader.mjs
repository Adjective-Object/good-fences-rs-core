import { registerHooks } from 'node:module';
import { readFileSync } from 'node:fs';

const parse = JSON.parse, hasOwn = Object.hasOwn, Url = URL, LoadError = Error;
const read = readFileSync;
const get = Function.call.bind(Map.prototype.get);
const has = Function.call.bind(Map.prototype.has);
const set = Function.call.bind(Map.prototype.set);
const modules = new Map(Object.entries(config.modules));
const urls = new Map([...modules.keys()].map(token => [
  token, `good-fences:${new Url(`native/${token.slice('good-fences:'.length)}.mjs`, import.meta.url).href}`,
]));
const sources = new Map([...modules].map(([token, pieces]) => [get(urls, token), pieces]));
const payloads = new Map();

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (has(sources, specifier)) return { url: specifier, shortCircuit: true };
    if (has(urls, specifier) && has(sources, context.parentURL)) {
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
        throw new LoadError(`Missing ESM chunk piece: ${filename} ${key}`);
      }
      source += payload[key];
    }
    return { format: 'module', source, shortCircuit: true };
  },
});

export async function loadEntry(name) {
  if (!hasOwn(config.entries, name)) throw new LoadError(`Unknown bundle entry: ${name}`);
  const result = [];
  const tokens = config.entries[name];
  for (let i = 0; i < tokens.length; i++) result[result.length] = await import(get(urls, tokens[i]));
  return result.length === 1 ? result[0] : result;
}
