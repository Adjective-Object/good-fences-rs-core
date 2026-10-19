import value, { object } from './shared.cjs';
import * as sync from './shared.cjs';
import later from './loader.cjs';
console.log('before', globalThis.effects);
const [a, b] = await Promise.all([import('./shared.cjs'), import('./shared.cjs')]);
console.log(a === b, a === sync, a.default === value && object === value.object);
console.log(later() === object, globalThis.effects);
