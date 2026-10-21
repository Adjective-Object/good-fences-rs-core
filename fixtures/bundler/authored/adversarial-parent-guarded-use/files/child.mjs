trace.push('child');
const multiplier = 3;
export function calculate(value) {
  trace.push('call:' + value);
  if (value === 0) throw new TypeError('zero');
  return value * multiplier;
}
