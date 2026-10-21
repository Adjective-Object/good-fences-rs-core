import * as state from './tla.mjs';
trace.push('root-b');
export function read() { state.bump(); return [state, state.receiver()]; }
