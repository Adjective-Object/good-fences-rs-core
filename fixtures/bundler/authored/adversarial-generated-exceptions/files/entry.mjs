globalThis.events.push('eager');
function fail(value) {
  globalThis.events.push('throw');
  throw value;
}
export async function caught(value, gates) {
  try {
    globalThis.events.push('try:start');
    await gates.body;
    fail(value);
  } catch (error) {
    globalThis.events.push('catch');
    return {error};
  } finally {
    globalThis.events.push('finally:start');
    await gates.finally;
    globalThis.events.push('finally:end');
  }
}
export async function replaced(value, replacement, gates) {
  try { return fail(value); }
  finally {
    globalThis.events.push('replace');
    await gates.finally;
    throw replacement;
  }
}
