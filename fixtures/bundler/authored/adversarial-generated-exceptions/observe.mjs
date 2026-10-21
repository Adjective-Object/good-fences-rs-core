import assert from 'node:assert/strict';
import {relative} from 'node:path';
import {pathToFileURL} from 'node:url';

function gate() {
  let release;
  let entered;
  const waiting = new Promise(resolve => { entered = resolve; });
  return {
    waiting,
    value: {then(resolve) { assert.equal(release, undefined); release = resolve; entered(); }},
    release() { assert.equal(typeof release, 'function'); release(); },
  };
}

export async function observe({entries, entry, mode, generatedBoundaries, directory, moduleMembers, segmentMembers}) {
  if (mode === 'bundled') {
    assert.equal(generatedBoundaries, 2);
    const artifact = relative(directory, entries[entry]);
    const source = moduleMembers.find(member => member.artifact === artifact)?.source;
    assert.equal(typeof source, 'string');
    const owner = index => {
      const member = segmentMembers.find(member => member.source === source && member.segment === index);
      assert.ok(member, `missing source segment ${index}`);
      return member.artifact;
    };
    assert.equal(owner(0), artifact);
    assert.notEqual(owner(1), artifact, 'fail must be separately owned');
    assert.equal(owner(2), artifact);
    assert.equal(owner(3), artifact);
  }
  globalThis.events = [];
  const ns = await import(pathToFileURL(entries[entry]).href);
  assert.deepEqual(events, ['eager']);
  const object = {marker: 7};
  const expected = ['eager'];
  for (const value of [undefined, object]) {
    const body = gate();
    const final = gate();
    let completed = false;
    const pending = ns.caught(value, {body: body.value, finally: final.value})
      .then(result => { completed = true; return result; });
    await body.waiting;
    await Promise.resolve();
    expected.push('try:start');
    assert.deepEqual(events, expected, 'body effects must wait for the original try await');
    assert.equal(completed, false);
    body.release();
    await final.waiting;
    await Promise.resolve();
    expected.push('throw', 'catch', 'finally:start');
    assert.deepEqual(events, expected, 'finally completion must wait for its original await');
    assert.equal(completed, false);
    final.release();
    const result = await pending;
    expected.push('finally:end');
    assert.deepEqual(events, expected);
    assert.equal(completed, true);
    assert.equal(Object.hasOwn(result, 'error'), true);
    assert.equal(result.error, value);
  }
  const replacement = {replacement: true};
  const final = gate();
  let completed = false;
  const pending = ns.replaced(undefined, replacement, {finally: final.value})
    .then(
      value => { completed = true; return {value}; },
      error => { completed = true; return {error}; },
    );
  await final.waiting;
  await Promise.resolve();
  expected.push('throw', 'replace');
  assert.deepEqual(events, expected);
  assert.equal(completed, false, 'replacement throw must wait for the original finally await');
  final.release();
  const outcome = await pending;
  assert.equal(completed, true);
  assert.equal(Object.hasOwn(outcome, 'error'), true);
  assert.equal(outcome.error, replacement);
  assert.deepEqual(events, expected);
  console.log('exceptions: undefined, object identity, awaited finally');
}
