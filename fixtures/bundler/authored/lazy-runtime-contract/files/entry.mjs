import {Module as M, Read as R, Action as A} from './runtime-barrel.mjs';
import {trace} from './trace.mjs';
const Ctor = M;
const captured = 'original-options';
function optionsEffect() { trace.push('options'); return captured; }
const originalModule = new Ctor(() => import('./certified.mjs'), {label: optionsEffect()});
const nestedModule = new Ctor(() => import('./certified.mjs'), {label: captured});
export const a = new R(originalModule, ns => ns.first, () => trace.push('govern'));
export const same = new R(originalModule, ns => ns.first);
export const b = new A(originalModule, ns => ns.second, {tag: 'action-extra'});
export function make(Ctor, extra) {
  return new R(nestedModule, ns => ns.first, () => trace.push(Ctor), extra);
}
const initialized = new M(() => import('./initialized.mjs'), {
  initializer: ns => trace.push('initializer:' + ns.extra())
});
export const fallback = new R(initialized, ns => ns.value);
const escaped = new M(() => import('./escaped.mjs'));
globalThis.escapedLazyModule = escaped;
export const escapedRead = new R(escaped, ns => ns.value);
class Untrusted extends M {}
const uncertified = new Untrusted(() => import('./untrusted.mjs'));
export const untrustedRead = new R(uncertified, ns => ns.value);
console.log('before', trace.join(','));
const nested = make('nested', 'genuine');
await nested.getValue();
const value = await a.getValue();
const identical = await a.getValue() === value && await same.getValue() === value;
console.log('reads', value(), identical, a.selections, nested.extra);
console.log('action', await b.execute(3), b.calls, b.options.tag);
console.log('fallback', (await fallback.getValue())(), (await escapedRead.getValue())(), (await untrustedRead.getValue())());
console.log('trace', trace.join(','));
