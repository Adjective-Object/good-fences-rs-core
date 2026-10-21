trace.push('child');
const secret = { count: 1 };
let value = 1;
export { value, value as alias };
export function read() { return secret; }
export function bump(amount) { value += amount; secret.count = value; }
