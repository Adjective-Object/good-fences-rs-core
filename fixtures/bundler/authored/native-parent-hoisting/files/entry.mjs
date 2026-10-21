globalThis.trace = [];

async function guarded(enabled) {
  if (enabled) await import('./guarded.mjs');
}

async function failure() {
  try {
    return (await import('./failure.mjs')).value;
  } catch (error) {
    return error === undefined;
  }
}

async function run() {
  await guarded(false);
  console.log('before', trace.length);
  const pending = import('./child.mjs').then(child => child.value);
  console.log('pending', trace.length);
  const child = await import('./child.mjs');
  const original = child.read;
  const secret = original();
  const first = await pending;
  child.bump();
  console.log('value', first, child.value, original === child.read, secret === child.read());
  await import('./empty.mjs');
  console.log('empty done');
  const results = await Promise.all([failure(), failure()]);
  console.log('failure', ...results, await failure());
  console.log('trace', trace.join(','));
}

run().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
