module.exports = {
  object: Object,
  create: Object.create,
  defineProperty: Object.defineProperty,
  getOwnPropertyNames: Object.getOwnPropertyNames,
  freeze: Object.freeze,
  toStringTag: Symbol.toStringTag,
  assign: Object.assign,
  Error,
  apply: Reflect.apply,
  then: Promise.prototype.then,
};
