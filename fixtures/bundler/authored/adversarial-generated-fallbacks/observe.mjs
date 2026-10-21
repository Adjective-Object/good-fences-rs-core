import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 1, 'only eligible may acquire a generated boundary');
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    for (const index of [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      assert.equal(member.artifact, artifact, 'fallback declarations and eligible remain co-located');
    }
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  assert.deepEqual(ns.sync(), [2]);
  assert.equal(globalThis.escaped(3), 5);
  assert.equal(ns.publicCall(), 7);
  assert.deepEqual(events, ['eager', 'callback', 'escaped', 'public']);
  const pending = ns.run();
  if (mode === 'bundled') {
    assert.deepEqual(events, ['eager', 'callback', 'escaped', 'public', 'capture', 'optional', 'public'],
      'capture, optional and public calls remain synchronous; only eligible suspends');
  }
  assert.deepEqual(await pending, [5, 6, 7, 8]);
  assert.deepEqual(events, ['eager', 'callback', 'escaped', 'public', 'capture', 'optional', 'public', 'eligible']);
  console.log('fallbacks: sync callback, escape, capture, optional, public');
}
