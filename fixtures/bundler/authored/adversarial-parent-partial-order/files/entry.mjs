async function run() {
  globalThis.trace = [];
  async function load() { return (await import('./parent.mjs')).value; }
  const reject = promise => promise.then(() => false, error => error === 'broken');
  console.log('cold', trace.length);
  console.log('failed', ...await Promise.all([reject(load()), reject(load())]), await reject(load()));
  console.log('partial', trace.join(','));
  console.log('later', (await import('./survivor.mjs')).value, trace.join(','));
  console.log('retry', await reject(load()), trace.join(','));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
