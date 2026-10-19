globalThis.attempts = (globalThis.attempts || 0) + 1;
console.log('attempt', globalThis.attempts);
throw Error('attempt ' + globalThis.attempts);
