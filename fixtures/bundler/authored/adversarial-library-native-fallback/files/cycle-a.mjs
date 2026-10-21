import {b} from './cycle-b.mjs';
trace.push('cycle-a');
export function a() { return 'A'; }
export function pair() { return [a(), b()]; }
