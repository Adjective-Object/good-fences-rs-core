import retry from './retry.cjs';
console.log('before');
const [a, b] = await Promise.all([
  import('./fail.cjs').catch(error => error),
  import('./fail.cjs').catch(error => error),
]);
const again = await import('./fail.cjs').catch(error => error);
console.log(a === b, a === again, globalThis.attempts);
console.log('require', retry());
