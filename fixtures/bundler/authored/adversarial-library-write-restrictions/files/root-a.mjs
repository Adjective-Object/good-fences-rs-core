import {writeConst, read} from './state.mjs';
export function readResult() { return [writeConst(), read]; }
export {readResult as read};
