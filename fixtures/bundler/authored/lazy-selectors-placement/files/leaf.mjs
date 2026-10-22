function sharedHelper() { return 4; }
export function first() { return sharedHelper(); }
export function second(value) { return sharedHelper() + value; }
export function unused() { return 'UNUSED_LAZY_IMPLEMENTATION'; }
