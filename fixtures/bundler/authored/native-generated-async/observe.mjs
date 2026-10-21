import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry}) {
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  assert.equal(await ns.run(false), false);
  assert.deepEqual(events, ['eager']);
  const pending = ns.run(true);
  assert.deepEqual(events, ['eager']);
  const [value, receiver, helper] = await pending;
  assert.equal(value, 7);
  assert.equal(receiver, true);
  const pair = await Promise.all([ns.identity(), ns.identity()]);
  assert.equal(pair[0].helper, helper);
  assert.equal(pair[1].helper, helper);
  assert.deepEqual(events, ['eager', 'body']);
  console.log('generated async: eager, guarded, suspended, stable');
}
