globalThis.events.push('eager');
function token(value) {
  globalThis.events.push(value);
  return value;
}
export async function run(value) {
  return [
    value && token('and'),
    value || token('or'),
    value ?? token('nullish'),
    value ? token('yes') : token('no'),
  ];
}
export async function identity() { return {token}; }
