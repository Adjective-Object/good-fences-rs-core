globalThis.observeActivation('outer');
export function later() { return import('./inner.mjs'); }
