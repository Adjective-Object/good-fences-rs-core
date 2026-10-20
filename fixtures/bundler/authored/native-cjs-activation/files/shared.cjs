var __filename;
var __dirname;
console.log('paths', typeof __filename, typeof __dirname, __filename.startsWith(__dirname));
globalThis.effects = (globalThis.effects || 0) + 1;
globalThis.observe = label => console.log(label);
globalThis.observe('shared');
exports.object = {};
