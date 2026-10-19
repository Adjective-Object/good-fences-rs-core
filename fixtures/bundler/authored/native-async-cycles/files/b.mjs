import { read } from './a.mjs';
console.log('b-before', read());
await 0;
console.log('b-after');
export const other = 4;
