globalThis.trace = [];
export function loadPair() {
  return import('./barrel.mjs').then(({ selectedA: a, selectedB: b }) => [a, b]);
}
console.log('before', trace.length);
const pending = loadPair();
console.log('pending', trace.length);
const [a, b] = await pending;
console.log('selected', a(), b(), trace.join(','));
const again = await loadPair();
console.log('again', a === again[0], b === again[1], trace.join(','));
