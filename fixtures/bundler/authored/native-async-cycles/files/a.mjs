import { other } from './b.mjs';
console.log('a-before', other);
export function read() { return 'hoisted'; }
export const value = await Promise.resolve(other + 1);
console.log('a-after');
