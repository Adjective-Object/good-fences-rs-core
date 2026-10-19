import {snapshot, value, bump, live, step} from './barrel.mjs';
import expression from './expression.mjs';
bump();
step();
console.log(snapshot, value, live, expression);
export {snapshot as 'snapshot-key', live as default};
