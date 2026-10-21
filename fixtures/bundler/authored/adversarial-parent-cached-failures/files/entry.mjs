async function run() {
  globalThis.trace = [];
  globalThis.failureToken = { reason: 'shared' };
  globalThis.referenceToken = new ReferenceError('not a TDZ');
  const rejected = (promise, token) => promise.then(() => false, error => error === token);
  async function undefinedFailure() { return (await import('./undefined.mjs')).value; }
  async function nullFailure() { return (await import('./null.mjs')).value; }
  async function objectFailure() { return (await import('./object.mjs')).value; }
  async function referenceFailure() { return (await import('./reference.mjs')).value; }
  async function check(name, load, token) {
    const results = await Promise.all([rejected(load(), token), rejected(load(), token)]);
    console.log(name, ...results, await rejected(load(), token), trace.filter(value => value === name).length);
  }
  console.log('cold', trace.length);
  await check('undefined', undefinedFailure, undefined);
  await check('null', nullFailure, null);
  await check('object', objectFailure, failureToken);
  await check('reference', referenceFailure, referenceToken);
  console.log('trace', trace.join(','));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
