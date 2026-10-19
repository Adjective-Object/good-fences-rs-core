globalThis.effects = (globalThis.effects || 0) + 1;
globalThis.observe = label => console.log(label);
globalThis.observe('shared');
exports.object = {};
