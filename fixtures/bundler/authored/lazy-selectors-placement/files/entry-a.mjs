import {a} from './selectors.mjs';
import {trace} from './trace.mjs';
globalThis.lazyA = a;
globalThis.lazyTrace = trace;
console.log('a-ready', trace.length);
