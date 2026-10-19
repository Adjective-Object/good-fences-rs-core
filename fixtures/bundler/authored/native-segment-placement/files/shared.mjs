console.log('shared:init');
export const leftValue = 11;
export const rightValue = 22;
export const identity = { token: 'shared' };
export let value = 0;
export function bump() {
  return ++value;
}
