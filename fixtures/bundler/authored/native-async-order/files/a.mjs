globalThis.observe('a-before');
await Promise.resolve();
globalThis.observe('a-after');
export const a = 1;
