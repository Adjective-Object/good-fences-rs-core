import {readThen as then, __proto__, init, getIdentity} from './y.mjs';
export function read() {
  if (then !== 5 || __proto__() !== 'proto-y' || init() !== 'init-y') throw new Error('y changed');
  return getIdentity();
}
