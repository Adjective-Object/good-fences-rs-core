globalThis.trace = [];
export async function loadSelected() {
  return (await import('./barrel.mjs')).selectedA;
}
console.log('before', trace.length);
const pending = loadSelected();
console.log('pending', trace.length);
const selected = await pending;
console.log('selected', selected(), trace.join(','));
console.log('again', selected === await loadSelected(), trace.join(','));
