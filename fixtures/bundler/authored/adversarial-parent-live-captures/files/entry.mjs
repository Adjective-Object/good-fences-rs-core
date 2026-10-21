async function run() {
  globalThis.trace = [];
  console.log('cold', trace.length);
  const pending = import('./child.mjs').then(child => child.read);
  const child = await import('./child.mjs');
  const originalRead = child.read;
  const object = originalRead();
  console.log('initial', child.value, child.alias, originalRead === await pending, originalRead === child.read, object === child.read());
  child.bump(4);
  console.log('mutated', child.value, child.alias, object.count, originalRead === child.read, object === originalRead());
  const again = await import('./child.mjs');
  console.log('repeat', again.value, again.alias, again.read === originalRead, again.read() === object, trace.join(','));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
