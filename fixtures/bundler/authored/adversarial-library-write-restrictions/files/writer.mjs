import {count} from './state.mjs';
trace.push('writer');
export function writeImport() {
  try { count = 99; } catch (error) { return error.name; }
  return 'write unexpectedly succeeded';
}
