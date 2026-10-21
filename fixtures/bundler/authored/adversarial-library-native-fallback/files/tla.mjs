trace.push('tla:start');
await Promise.resolve();
trace.push('tla:end');
export let count = 1;
export function bump() { count++; }
export function receiver() { return this.count; }
