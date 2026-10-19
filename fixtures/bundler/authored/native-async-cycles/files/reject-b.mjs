import './reject-a.mjs';
globalThis.attempts = (globalThis.attempts || 0) + 1;
globalThis.observe('reject once');
await Promise.reject(new RangeError('cached failure'));
console.log('unreachable b');
