trace.push('x');
const identity = {};
let count = 0;
export function sampleX() {
  count++;
  return {identity, count, read: () => count, bump: amount => { count += amount; }};
}
