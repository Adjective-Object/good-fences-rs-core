trace.push('then-init');
export const value = 8;
export function then(resolve) { trace.push('then-call'); resolve({ value }); }
