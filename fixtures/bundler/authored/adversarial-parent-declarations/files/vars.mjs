trace.push('vars');
var value = 2;
var value = value + 5;
export { value };
export function read() { return value; }
export function nested() {
  var local;
  function local() { return value; }
  var local;
  return local() === value;
}
