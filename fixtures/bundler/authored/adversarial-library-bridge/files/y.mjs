trace.push('y');
const identity = {};
let count = 0;
export function sampleY() {
  count++;
  return {identity, count};
}
