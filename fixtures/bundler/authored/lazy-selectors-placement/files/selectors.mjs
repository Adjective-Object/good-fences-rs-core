import {Module as M, Read as R, Action as A} from './runtime-barrel.mjs';
import {trace} from './trace.mjs';
const captured = 'captured';
const optionValue = captured;
const originalModule = new M(() => import('./barrel.mjs'), {
  label: optionValue, onLoad: () => trace.push(captured)
});
export const a = new R(originalModule, ns => ns.first, () => trace.push('govern'), 'read-extra');
export const b = new A(originalModule, ns => ns.second, {tag: 'action-extra'});
const unusedSelector = new R(originalModule, ns => ns.unused);
