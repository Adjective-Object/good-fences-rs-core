trace.push('x');
const secret = {};
export let count = 0;
export function readX() {
  count++;
  return [secret, readX, count];
}
