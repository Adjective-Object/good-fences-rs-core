import {a} from './cycle-a.mjs';
trace.push('cycle-b');
export function b() { return a() === 'A' ? 'B' : 'wrong'; }
