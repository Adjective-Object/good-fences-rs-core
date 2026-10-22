import {trace} from './trace.mjs';
trace.push('certified');
export function first() { return 4; }
export function second(value) { return 4 + value; }
export function unused() { return 'UNUSED_CERTIFIED_IMPLEMENTATION'; }
