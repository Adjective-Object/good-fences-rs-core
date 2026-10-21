export default {
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
  slice: String.prototype.slice,
  lastIndexOf: String.prototype.lastIndexOf,
};
