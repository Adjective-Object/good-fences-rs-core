import { a } from './a.mjs';
import { b } from './b.mjs';
globalThis.observe('job');
export { a, b };
export function later() { return import('./inner.mjs'); }
