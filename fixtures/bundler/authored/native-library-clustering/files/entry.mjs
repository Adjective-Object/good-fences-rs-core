async function main() {
  globalThis.trace = [];
  const a = async () => (await import('./root-a.mjs')).read();
  const b = async () => (await import('./root-b.mjs')).read();
  const c = async () => (await import('./root-c.mjs')).read();
  console.log('before', trace.length);
  const pending = a();
  console.log('pending', trace.length);
  const first = await pending;
  console.log('first', trace.join(','), first[2]);
  const both = await b();
  console.log('shared', first[0] === both[0][0], first[1] === both[0][1], both[0][2], await c());
  const fail = async () => {
    try { await import('./failure.mjs'); return false; }
    catch (error) { return error === undefined; }
  };
  console.log('failure', ...await Promise.all([fail(), fail()]), await fail());
  console.log('trace', trace.join(','));
}
main();
