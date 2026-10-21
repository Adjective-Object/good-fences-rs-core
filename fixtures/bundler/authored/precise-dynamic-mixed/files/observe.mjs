import * as ns from './barrel.mjs';
console.log('public', Object.keys(ns).join(','), ns.value);
ns.change();
console.log('public live', ns.value, ns.other());
