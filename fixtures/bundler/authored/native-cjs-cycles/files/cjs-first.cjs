console.log('cjs before');
exports.value = 3;
const esm = require('./dep.mjs');
console.log(esm.read() === exports, esm.read().value);
