globalThis.trace = ['shared'];
export const object = { count: 0 };
export let count = 0;
export function bump() { count++; object.count = count; }
