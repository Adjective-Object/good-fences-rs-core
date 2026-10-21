async function run() {
  globalThis.trace = ['entry'];
  const value = 'entry';
  function read() { return value; }
  console.log('cold', read());
  await import('./empty.mjs');
  console.log('empty', trace.join(','));
  const firstPlain = (await import('./plain.mjs')).unique;
  console.log('plain', firstPlain(), firstPlain === (await import('./plain.mjs')).unique);
  const left = await import('./left.mjs');
  const right = await import('./right.mjs');
  const leftRead = left.read;
  const rightRead = right.read;
  console.log('functions', leftRead(), rightRead(), leftRead.name, rightRead.name, leftRead === (await import('./left.mjs')).read, rightRead === (await import('./right.mjs')).read);
  const vars = await import('./vars.mjs');
  console.log('vars', vars.value, vars.read(), vars.read === (await import('./vars.mjs')).read, vars.nested(), trace.join(','));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
