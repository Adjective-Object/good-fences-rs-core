import {readX} from './lib-x.mjs';
import {readY} from './lib-y.mjs';
export function read() {
  const left = readX();
  const right = readY();
  if (left.length !== 3 || typeof left[0] !== 'object' || typeof left[1] !== 'function') {
    throw new Error('shared binding result shape changed');
  }
  if (right !== 7) throw new Error('second island was not initialized');
  return [left, right];
}
