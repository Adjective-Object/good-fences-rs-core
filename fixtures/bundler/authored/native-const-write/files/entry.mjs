import {value as live} from './dep.mjs';
try { before; } catch (error) { console.log(error.name); }
const before = 1;
const fixed = 2;
try { fixed = 3; } catch (error) { console.log(error.name); }
console.log(live);
console.log('before const write');
fixed = 3;
console.log('unreachable');
