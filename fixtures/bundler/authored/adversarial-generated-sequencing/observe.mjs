import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 1);
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    const owner = index => {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      return member.artifact;
    };
    assert.equal(owner(0), artifact);
    assert.notEqual(owner(1), artifact, 'target must be separately owned');
    assert.equal(owner(2), artifact);
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  const pending = ns.run({
    get first() { events.push('get:first'); return 1; },
    get rest() {
      events.push('get:rest');
      return {
        [Symbol.iterator]() {
          events.push('iterator');
          let index = 0;
          return {next() {
            events.push(`next:${index}`);
            return index++ === 0 ? {value: 2, done: false} : {done: true};
          }};
        },
      };
    },
    async last() { events.push('last:start'); await 0; events.push('last:end'); return 3; },
  });
  if (mode === 'bundled') {
    assert.ok(generatedBoundaries > 0, 'suspension check requires extracted references');
    assert.deepEqual(events, ['eager', 'before']);
  }
  const result = await pending;
  assert.deepEqual(result, {receiver: true, args: [1, 2, 3]});
  assert.deepEqual(events, [
    'eager', 'before', 'get:first', 'get:rest', 'iterator', 'next:0', 'next:1',
    'last:start', 'last:end', 'body', 'after',
  ]);
  const thrown = {argument: true};
  let rejected = false;
  try {
    await ns.run({
      get first() { events.push('get:throw'); throw thrown; },
      get rest() { assert.fail('spread must not follow an abrupt first argument'); },
    });
  } catch (error) { rejected = true; assert.equal(error, thrown); }
  assert.equal(rejected, true);
  rejected = false;
  try {
    await ns.run({
      first: 1,
      rest: {[Symbol.iterator]() {
        events.push('iterator:throw');
        return {next() { throw undefined; }};
      }},
      last() { assert.fail('last argument must not follow abrupt iteration'); },
    });
  } catch (error) { rejected = true; assert.equal(error, undefined); }
  assert.equal(rejected, true);
  assert.deepEqual(events, [
    'eager', 'before', 'get:first', 'get:rest', 'iterator', 'next:0', 'next:1',
    'last:start', 'last:end', 'body', 'after', 'before', 'get:throw',
    'before', 'iterator:throw',
  ]);
  console.log('sequencing: receiver, getters, iterator, nested await');
}
