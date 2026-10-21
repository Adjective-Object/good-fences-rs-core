globalThis.events.push('eager');
function target(...args) {
  globalThis.events.push('body');
  return {receiver: this === undefined, args};
}
export async function run(input) {
  globalThis.events.push('before');
  const result = target(input.first, ...input.rest, await input.last());
  globalThis.events.push('after');
  return result;
}
