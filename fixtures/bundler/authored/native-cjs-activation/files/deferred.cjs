globalThis.effects++;
globalThis.observe('deferred');
const original = exports;
exports = {};
module.exports = this === original ? require('./shared.cjs').object : null;
