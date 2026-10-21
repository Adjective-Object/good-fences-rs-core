import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 3);
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    for (const index of [0, 1, 2, 3, 4]) {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      assert.equal(member.artifact, artifact, 'mutually recursive even/odd must share the entry owner');
    }
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  const identities = await Promise.all([ns.identity(), ns.identity(), ns.identity()]);
  assert.deepEqual(events, ['eager']);
  for (const pair of identities) {
    assert.equal(pair.even, identities[0].even);
    assert.equal(pair.odd, identities[0].odd);
    assert.notEqual(pair.even, pair.odd);
  }
  const pending = ns.run(4);
  if (mode === 'bundled') {
    assert.ok(generatedBoundaries > 0, 'suspension check requires extracted references');
    assert.deepEqual(events, ['eager']);
  }
  assert.equal(await pending, identities[0].even);
  assert.deepEqual(events, ['eager', 'even:4', 'odd:3', 'even:2', 'odd:1', 'even:0']);
  const pair = await Promise.all([ns.run(0), ns.run(0)]);
  assert.equal(pair[0], identities[0].even);
  assert.equal(pair[1], identities[0].even);
  assert.deepEqual(events, ['eager', 'even:4', 'odd:3', 'even:2', 'odd:1', 'even:0', 'even:0', 'even:0']);
  assert.equal(identities[0].odd(0), identities[0].odd);
  console.log('recursion: synchronous internals, concurrent stable identities');
}
