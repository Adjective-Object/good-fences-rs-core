globalThis.events.push('eager');
function Promise(value) {
  globalThis.events.push('Promise:body');
  return value + 1;
}
function then(value) {
  globalThis.events.push('then:body');
  return value * 2;
}
export async function run(input) {
  const first = Promise(input.first);
  return then(first);
}
export async function identity() { return {functions: {Promise, then}}; }
