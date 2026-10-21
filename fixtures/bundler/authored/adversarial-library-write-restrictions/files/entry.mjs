globalThis.trace = [];
console.log('cold', trace.length);
const {read: a} = await import('./root-a.mjs');
const first = a();
console.log('a', trace.join(','), first[0], first[1]());
const {read: b} = await import('./root-b.mjs');
console.log('b', trace.join(','), ...b(), first[1]());
const {read: c} = await import('./root-c.mjs');
console.log('c', trace.join(','), c(), first[1]());
