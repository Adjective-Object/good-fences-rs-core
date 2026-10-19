import { value } from './tdz-a.mjs';
console.log('before TDZ');
await 0;
console.log(value);
