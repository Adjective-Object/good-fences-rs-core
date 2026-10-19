globalThis.initializations = 0;
globalThis.observeInitialization = () => console.log('shared', ++globalThis.initializations);
console.log('before', globalThis.initializations);
const [a, again, b] = await Promise.all([import('./a.mjs'), import('./a.mjs'), import('./b.mjs')]);
console.log(a === again, a.shared === b.shared, a.box === b.box, a.bump === b.bump, globalThis.initializations);
a.bump();
console.log(a.box.count, b.box.count, b.shared.box.count);
const shared = await import('./shared.mjs');
console.log(shared === a.shared, globalThis.initializations);
