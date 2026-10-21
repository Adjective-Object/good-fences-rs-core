async function run() {
  globalThis.trace = [];
  async function choose(enabled) { return enabled && (await import('./never.mjs')).value; }
  async function delayed(value) {
    trace.push('enter');
    try { return (await import('./child.mjs')).calculate(value); }
    finally { trace.push('finally'); }
  }
  console.log('cold', await choose(false), trace.length);
  const saved = delayed;
  console.log('saved', trace.length);
  console.log('first', await saved(4), trace.join(','));
  trace.length = 0;
  try { await saved(0); } catch (error) { console.log('caught', error.name, trace.join(',')); }
  trace.length = 0;
  console.log('again', await saved(2), trace.join(','));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
