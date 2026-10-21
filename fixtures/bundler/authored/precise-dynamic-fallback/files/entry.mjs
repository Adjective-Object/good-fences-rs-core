globalThis.trace = [];
function escape(ns) { return ns; }
console.log('before', trace.length);
const ns = await import('./barrel.mjs').then(escape);
console.log('namespace', Object.keys(ns).join(','), ns.receiver(), ns.spare(), trace.join(','));
const result = await import('./barrel.mjs').then(ns => ns.receiver());
console.log('receiver', result, ns === await import('./barrel.mjs'));
const missing = (await import('./missing.mjs')).absent;
const missingAgain = await import('./missing.mjs').then(ns => ns.absent);
console.log('missing', missing === undefined, missingAgain === undefined, trace.join(','));
