import {readThen as then, __proto__, init, getIdentity} from './x.mjs';
export function read() {
  const result = [getIdentity(), then, __proto__(), init()];
  if (result[0] === null || typeof result[0] !== 'object' || result[1] !== 3) throw new Error('x changed');
  return result;
}
