import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 4);
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    const owner = index => {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      return member.artifact;
    };
    assert.equal(owner(0), artifact);
    assert.notEqual(owner(1), artifact, 'Promise must be separately owned');
    assert.notEqual(owner(2), artifact, 'then must be separately owned');
    assert.notEqual(owner(1), owner(2), 'independent functions retain separate owners with target zero');
    assert.equal(owner(3), artifact);
    assert.equal(owner(4), artifact);
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  assert.equal(await ns.run({get first() { events.push('argument'); return 3; }}), 8);
  assert.deepEqual(events, ['eager', 'argument', 'Promise:body', 'then:body']);
  const {functions: first} = await ns.identity();
  const {functions: second} = await ns.identity();
  assert.deepEqual(Object.keys(first), ['Promise', 'then']);
  assert.equal(typeof first.Promise, 'function');
  assert.equal(typeof first.then, 'function');
  assert.equal(first.Promise, second.Promise);
  assert.equal(first.then, second.then);
  assert.deepEqual(events, ['eager', 'argument', 'Promise:body', 'then:body']);
  assert.equal(first.Promise(10), 11);
  assert.equal(first.then(10), 20);
  assert.deepEqual(events, ['eager', 'argument', 'Promise:body', 'then:body', 'Promise:body', 'then:body']);
  console.log('names: Promise shadow, then export, argument effects');
}
