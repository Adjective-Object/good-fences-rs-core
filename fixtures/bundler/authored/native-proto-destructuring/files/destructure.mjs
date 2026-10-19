import {__proto__ as other} from './dep.mjs';
let {__proto__ = null} = {['__proto__']: null};
console.log(other, Object.keys({__proto__}), Object.getPrototypeOf({__proto__}) === Object.prototype, Object.getPrototypeOf({__proto__: __proto__}) === null);
({__proto__ = 4} = {});
console.log(__proto__ === Object.prototype);
