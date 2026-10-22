import {b} from './selectors.mjs';
import {trace} from './trace.mjs';
globalThis.lazyB = b;
console.log('b-ready', trace.length);
