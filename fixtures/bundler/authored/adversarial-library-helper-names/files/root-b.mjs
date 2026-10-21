import {readThen as xThen, __proto__ as xProto, init as xInit, getIdentity as xIdentity} from './x.mjs';
import {readThen as then, __proto__, init, getIdentity} from './y.mjs';
export function read() {
  if (xThen !== 3 || xProto() !== 'proto-x' || xInit() !== 'init-x') throw new Error('x changed');
  const result = [xIdentity(), getIdentity(), then, __proto__(), init()];
  if (result[1] === null || typeof result[1] !== 'object' || result[2] !== 5) throw new Error('y changed');
  return result;
}
