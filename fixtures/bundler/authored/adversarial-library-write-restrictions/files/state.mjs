trace.push('state');
const fixed = 8;
export let count = 4;
export function bump() { count++; }
export function read() { return count; }
export function writeConst() {
  try { fixed = 9; } catch (error) { return error.name; }
  return 'write unexpectedly succeeded';
}
