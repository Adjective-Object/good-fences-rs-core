function sharedHelper() { return 10; }
function onlyA() { return 1; }
function onlyB() { return 2; }
export function alpha() { return sharedHelper() + onlyA(); }
export function beta() { return sharedHelper() + onlyB(); }
export function initHelper() { return sharedHelper() + 3; }
export function unusedSibling() { return import('./dead.mjs'); }
