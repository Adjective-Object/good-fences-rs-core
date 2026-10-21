globalThis.events.push('eager');

function helper(value) {
  globalThis.events.push('body');
  return [value, this === undefined, helper];
}

export async function run(enabled) {
  return enabled && helper(7);
}

export async function identity() {
  return {helper};
}
