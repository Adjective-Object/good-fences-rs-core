import {pair} from './cycle-a.mjs';
import * as state from './tla.mjs';
trace.push('root-a');
export function read() {
  return [...pair(), state.receiver(), Object.keys(state).join(',') === 'bump,count,receiver', state];
}
