trace.push('z');
const identity = {};
let count = 0;
export function sampleZ() {
  count++;
  return {identity, count, read: () => count, bump: amount => { count += amount; }};
}
