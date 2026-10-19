import {Same, Model} from './declarations.mjs';
import ordinary from './function.mjs';
import asynchronous from './async.mjs';
import generator from './generator.mjs';
import Class from './class.mjs';
import arrow from './arrow.mjs';
import parenthesized from './parenthesized.mjs';
import comma from './comma.mjs';
import conditional from './conditional.mjs';
const fn = () => Same;
const object = {Same, Model, fn};
console.log(object.Same.name, object.Model.name, object.fn.name, fn() === Same);
for (const value of [ordinary, asynchronous, generator, Class, arrow, parenthesized, comma, conditional]) {
  const descriptor = Object.getOwnPropertyDescriptor(value, 'name');
  console.log(value.name, object.fn.name, descriptor.writable, descriptor.enumerable, descriptor.configurable);
}
console.log(Class.seen);
