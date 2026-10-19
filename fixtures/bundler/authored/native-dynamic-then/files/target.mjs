console.log('target');
export let value = 1;
export default value;
export function bump() { value++; }
export function then(resolve) {
  console.log('then', this.bump === bump);
  bump();
  resolve(bump);
}
