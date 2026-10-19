import * as subset from './subset.mjs';
import * as entry from './entry.mjs';
import * as dependency from './dep.mjs';
console.log('subset', Object.keys(subset).join(','), subset.x, 'y' in subset, 'default' in subset);
console.log('combined', Object.keys(entry).join(','), entry.x, entry.y, 'default' in entry);
console.log('dependency', Object.keys(dependency).join(','));
