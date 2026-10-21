import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 6);
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    for (const index of [0, 1, 2, 3]) {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      assert.equal(member.artifact, artifact, 'token and its consumers must be co-located');
    }
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  assert.deepEqual(await ns.run(false), [false, 'or', false, 'no']);
  assert.deepEqual(events, ['eager', 'or', 'no']);
  assert.deepEqual(await ns.run(0), [0, 'or', 0, 'no']);
  assert.deepEqual(await ns.run(null), [null, 'or', 'nullish', 'no']);
  assert.deepEqual(await ns.run('kept'), ['and', 'kept', 'kept', 'yes']);
  assert.deepEqual(events, ['eager', 'or', 'no', 'or', 'no', 'or', 'nullish', 'no', 'and', 'yes']);
  const [first, second] = await Promise.all([ns.identity(), ns.identity()]);
  assert.deepEqual(Object.keys(first), ['token']);
  assert.equal(first.token, second.token);
  assert.equal(first.token('outside'), 'outside');
  assert.deepEqual(events, ['eager', 'or', 'no', 'or', 'no', 'or', 'nullish', 'no', 'and', 'yes', 'outside']);
  console.log('guards: cold branches, exact values, shorthand identity');
}
