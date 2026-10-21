globalThis.events.push('eager');
function even(n) {
  globalThis.events.push(`even:${n}`);
  return n === 0 ? even : odd(n - 1);
}
function odd(n) {
  globalThis.events.push(`odd:${n}`);
  return n === 0 ? odd : even(n - 1);
}
export async function run(n) { return even(n); }
export async function identity() { return {even, odd}; }
