import {getIdentity as xIdentity} from './barrel-x.mjs';
import {then, __proto__, init, getIdentity} from './barrel-y.mjs';
export function read() { return [xIdentity(), getIdentity(), then(), __proto__(), init()]; }
