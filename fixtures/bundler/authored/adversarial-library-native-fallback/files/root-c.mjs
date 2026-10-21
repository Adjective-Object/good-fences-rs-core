import {pair} from './cycle-a.mjs';
import * as state from './tla.mjs';
trace.push('root-c');
export function read() {
  if (pair().join('') !== 'AB') throw new Error('cycle changed');
  return [state, state.receiver()];
}
