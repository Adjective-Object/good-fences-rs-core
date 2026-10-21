globalThis.events.push('eager');
const captured = 5;
function callback(value) { globalThis.events.push('callback'); return value + 1; }
function escaped(value) { globalThis.events.push('escaped'); return value + 2; }
globalThis.escaped = escaped;
function capture() { globalThis.events.push('capture'); return captured; }
function optional() { globalThis.events.push('optional'); return 6; }
function eligible(value) { globalThis.events.push('eligible'); return value * 2; }
export function publicCall() { globalThis.events.push('public'); return 7; }
export function sync() { return [1].map(value => callback(value)); }
export async function run() {
  return [capture(), optional?.(), publicCall(), eligible(4)];
}
