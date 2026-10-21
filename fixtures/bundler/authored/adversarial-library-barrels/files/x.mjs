trace.push('x');
const identity = {};
export function then() { return 'x:then'; }
export function __proto__() { return 'proto-x'; }
export function init() { return 'init-x'; }
export default function value() { return 'default-x'; }
export function getIdentity() { return identity; }
