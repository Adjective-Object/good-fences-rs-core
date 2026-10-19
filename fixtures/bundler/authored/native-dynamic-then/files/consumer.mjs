import * as namespace from './target.mjs';
export { namespace };
export function later() { return import('./target.mjs'); }
