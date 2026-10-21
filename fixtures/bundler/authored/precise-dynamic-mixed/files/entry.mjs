const value = (await import('./barrel.mjs')).value;
const ns = await import('./barrel.mjs');
console.log('selected', value, Object.keys(ns).join(','));
ns.change();
console.log('live', ns.value, ns.other());
