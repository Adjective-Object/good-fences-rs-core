globalThis.observe = value => console.log(value);
console.log('before');
const first = import('./reject-a.mjs');
console.log('pending');
const results = await Promise.allSettled([first, import('./reject-b.mjs'), import('./reject-a.mjs')]);
console.log(
  results.every(result => result.status === 'rejected'),
  results[0].reason === results[1].reason,
  results[0].reason === results[2].reason,
  results[0].reason instanceof RangeError,
  globalThis.attempts
);
try { await import('./reject-a.mjs'); }
catch (error) { console.log(error === results[0].reason, globalThis.attempts); }
