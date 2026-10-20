import * as classes from './classes.js';
import * as flow from './flow.js';

console.log(JSON.stringify(classes.read()));
console.log(JSON.stringify(flow.read()));
console.log(Object.keys(classes).join(','), Object.keys(flow).join(','));
