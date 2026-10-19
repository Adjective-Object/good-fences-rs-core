console.log(hoisted());
var value = 1;
var value = 2;
export {value as default};
export function increment() {
  function identity(value) { return value; }
  value += identity(1);
}
export function hoisted() { return later; }
var later = 7;
var later;
console.log(typeof key);
