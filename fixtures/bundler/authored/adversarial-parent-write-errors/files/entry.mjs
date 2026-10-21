async function run() {
  globalThis.trace = [];
  console.log('cold', trace.length);
  const own = await import('./const.mjs');
  try { own.write(); } catch (error) { console.log('const', error.name, own.value, trace.join(',')); }
  trace.length = 0;
  const imported = await import('./imported.mjs');
  try { imported.write(); } catch (error) { console.log('import', error.name, imported.read(), trace.join(',')); }
  trace.length = 0;
  const repeated = await import('./const.mjs');
  try { repeated.write(); } catch (error) { console.log('repeat', error.name, repeated.value, trace.join(',')); }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
