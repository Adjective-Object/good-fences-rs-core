trace.push('const-init');
export const value = 2;
export function write() { trace.push('const-before'); value = 9; trace.push('const-after'); }
