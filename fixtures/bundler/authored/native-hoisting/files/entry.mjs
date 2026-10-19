import value from './function.mjs';
import {original as current, bump as step, read} from './barrel.mjs';
console.log(value.name, value());
console.log(current, read());
step();
console.log(current);
