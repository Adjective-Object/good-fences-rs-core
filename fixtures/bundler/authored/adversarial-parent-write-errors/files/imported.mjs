import { value } from './dependency.mjs';
trace.push('import-init');
export function read() { return value; }
export function write() { trace.push('import-before'); value = 9; trace.push('import-after'); }
