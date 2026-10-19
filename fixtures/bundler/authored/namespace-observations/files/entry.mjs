import * as ns from './leaf.mjs';
import {nested as alias, value as live} from './barrel.mjs';
console.log(Object.keys(ns).join(','), ns.value);
console.log(ns === alias, Object.getPrototypeOf(ns) === null, Object.isExtensible(ns));
const descriptor = Object.getOwnPropertyDescriptor(ns, 'value');
console.log(descriptor.value, descriptor.writable, descriptor.enumerable, descriptor.configurable, Object.prototype.toString.call(ns));
console.log(ns.read());
globalThis.namespaceKey = () => Object.keys(ns).find(key => key === 'value');
const key = globalThis.namespaceKey();
ns.bump();
console.log(ns[key], alias.value, live, ns.bump === alias.bump);
try { ns.value = 9; } catch (error) { console.log(error.name); }
console.log(ns.value);
