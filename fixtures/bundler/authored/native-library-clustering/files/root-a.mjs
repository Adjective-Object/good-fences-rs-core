import {readX} from './lib-x.mjs';
export function read() {
  const result = readX();
  if (result.length !== 3 || typeof result[0] !== 'object' || typeof result[1] !== 'function') {
    throw new Error('shared binding result shape changed');
  }
  if (result[2] < 1) throw new Error('counter was not initialized');
  return result;
}
