import { leftValue, identity, value, bump } from './shared.mjs';
export const named = () => leftValue;
export const local = 7;
export { identity, value, bump };
console.log('left:init');
