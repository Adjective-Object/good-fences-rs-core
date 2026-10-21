globalThis.trace = [];
console.log('cold', trace.length);
const first = (await import('./root-a.mjs')).read();
console.log('a', trace.join(','), first[1], first[2], first[3]);
const both = (await import('./root-b.mjs')).read();
console.log('b', trace.join(','), first[0] === both[0], both[2], both[3], both[4]);
const last = (await import('./root-c.mjs')).read();
console.log('c', trace.join(','), both[1] === last);
