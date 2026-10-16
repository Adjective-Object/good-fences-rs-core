import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { discoverCases, runFixtures } from './bundler-fixtures.mjs';

function withCorpus(callback) {
  const directory = mkdtempSync(path.join(tmpdir(), 'bundler-runner-test-'));
  const root = path.join(directory, 'corpus');
  const tempRoot = path.join(directory, 'runs');
  mkdirSync(path.join(root, 'synthetic'), { recursive: true });
  mkdirSync(tempRoot);
  writeFileSync(path.join(root, 'synthetic', 'LICENSE'), 'Synthetic test license, not upstream test data.');
  try {
    callback({ root, tempRoot, run: (options = {}) => runFixtures(root, { tempRoot, ...options }) });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function addCase(root, name = 'example', { sources, ...overrides } = {}) {
  const directory = path.join(root, 'synthetic', name);
  mkdirSync(path.join(directory, 'files'), { recursive: true });
  const manifest = {
    id: `synthetic/${name}`,
    upstream: {
      repository: 'https://github.com/example/synthetic',
      commit: 'a'.repeat(40),
      paths: ['tests/example.js'],
      test: 'synthetic example',
      license: 'MIT',
      adaptations: [],
    },
    moduleType: 'module',
    entryPoints: ['entry.js'],
    expectations: { stdout: ['hello'] },
    goals: ['Exercise runner behavior with locally authored data.'],
    ...overrides,
  };
  writeFileSync(path.join(directory, 'case.json'), JSON.stringify(manifest));
  for (const [name, source] of Object.entries(sources ?? { 'entry.js': "console.log('hello');" })) {
    const filename = path.join(directory, 'files', name);
    mkdirSync(path.dirname(filename), { recursive: true });
    writeFileSync(filename, source);
  }
  return directory;
}

test('deterministic cases, ESM default, isolated copy/cwd, and cleanup on success', () => {
  withCorpus(({ root, tempRoot, run }) => {
    addCase(root, 'z-last');
    const directory = addCase(root, 'a-first', {
      sources: {
        'entry.js': `
          import { readFileSync, writeFileSync } from 'node:fs';
          if (JSON.parse(readFileSync('package.json')).type !== 'module') throw Error('type');
          console.log(readFileSync('data.txt', 'utf8'));
          writeFileSync('data.txt', 'changed');
        `,
        'data.txt': 'hello',
      },
    });
    assert.deepEqual(run().map(({ id }) => id), ['synthetic/a-first', 'synthetic/z-last']);
    assert.equal(readFileSync(path.join(directory, 'files/data.txt'), 'utf8'), 'hello');
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('entrypoints run separately in manifest order with exact concatenated lines', () => {
  withCorpus(({ root, run }) => {
    addCase(root, 'multi', {
      entryPoints: ['second.js', 'first.js'],
      expectations: { stdout: ['second', '', 'first'] },
      sources: {
        'second.js': "globalThis.leak = true; process.stdout.write('second\\r\\n\\r\\n');",
        'first.js': "if (globalThis.leak) throw Error('process leaked'); process.stdout.write('first');",
      },
    });
    assert.deepEqual(run()[0].stdout, ['second', '', 'first']);
  });
});

test('CommonJS default and supplied package metadata are respected without scripts', () => {
  withCorpus(({ root, run }) => {
    addCase(root, 'commonjs', {
      moduleType: 'commonjs',
      sources: { 'entry.js': "require('node:assert').equal(typeof module.exports, 'object'); console.log('hello');" },
    });
    addCase(root, 'explicit-package', {
      sources: {
        'package.json': JSON.stringify({ type: 'commonjs', scripts: { install: 'exit 1' }, marker: 42 }),
        'entry.js': "require('node:assert').equal(require('./package.json').marker, 42); console.log('hello');",
      },
    });
    assert.equal(run().length, 2);
  });
});

for (const [name, source, expected] of [
  ['output mismatch', "console.log('wrong');", /stdout mismatch/u],
  ['unexpected blank line', "console.log('hello\\n');", /stdout mismatch/u],
  ['nonzero exit', 'process.exit(7);', /exit 7/u],
  ['stderr', "console.error('warning'); console.log('hello');", /unexpected stderr/u],
  ['timeout', 'while (true) {}', /ETIMEDOUT/u],
  ['output limit', "process.stdout.write('x'.repeat(2 * 1024 * 1024));", /ENOBUFS/u],
  ['signal', "process.kill(process.pid, 'SIGKILL');", /abnormal termination/u],
  ['missing external dependency', "import 'not-a-real-fixture-package';", /abnormal termination/u],
]) {
  test(`${name} fails and cleans the copied case`, () => {
    withCorpus(({ root, tempRoot, run }) => {
      addCase(root, 'failure', { sources: { 'entry.js': source } });
      assert.throws(() => run({ timeoutMs: name === 'timeout' ? 100 : 5000 }), expected);
      assert.deepEqual(readdirSync(tempRoot), []);
    });
  });
}

test('silent contracts are valid but an empty corpus never passes, including CLI', () => {
  withCorpus(({ root, tempRoot, run }) => {
    assert.throws(run, /No fixture cases/u);
    const runner = fileURLToPath(new URL('./bundler-fixtures.mjs', import.meta.url));
    const result = spawnSync(process.execPath, [runner, root], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /No fixture cases/u);
    addCase(root, 'silent', { sources: { 'entry.js': '' }, expectations: { stdout: [] } });
    assert.deepEqual(run()[0].stdout, []);
    const success = spawnSync(process.execPath, [runner, root], {
      encoding: 'utf8', env: { ...process.env, TMPDIR: tempRoot, TEMP: tempRoot, TMP: tempRoot },
    });
    assert.equal(success.status, 0, success.stderr);
    assert.equal(success.stderr, '');
    assert.equal(success.stdout, 'PASS baseline synthetic/silent\nValidated 1 runtime semantic contracts; no bundler conformance tested.\n');
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

for (const [name, mutate, expected] of [
  ['missing field', (m) => { delete m.goals; }, /exactly/u],
  ['unknown requirement', (m) => { m.bundler = 'rspack'; }, /exactly/u],
  ['bad module type', (m) => { m.moduleType = 'typescript'; }, /moduleType/u],
  ['empty entrypoints', (m) => { m.entryPoints = []; }, /entryPoints/u],
  ['duplicate entrypoints', (m) => { m.entryPoints = ['entry.js', 'entry.js']; }, /unique/u],
  ['traversal', (m) => { m.entryPoints = ['../case.json']; }, /relative path/u],
  ['absolute path', (m) => { m.entryPoints = ['/entry.js']; }, /relative path/u],
  ['Windows path', (m) => { m.entryPoints = ['C:\\entry.js']; }, /relative path/u],
  ['missing entrypoint', (m) => { m.entryPoints = ['absent.js']; }, /ENOENT/u],
  ['bad id', (m) => { m.id = 'other/example'; }, /id must match/u],
  ['short commit', (m) => { m.upstream.commit = 'abc'; }, /commit/u],
  ['unsafe provenance path', (m) => { m.upstream.paths = ['../test.js']; }, /relative path/u],
  ['missing provenance', (m) => { delete m.upstream.license; }, /exactly/u],
  ['bad repository', (m) => { m.upstream.repository = 'http://example.com'; }, /repository/u],
  ['empty adaptations text', (m) => { m.upstream.adaptations = ['']; }, /nonempty text/u],
  ['multiline expectation', (m) => { m.expectations.stdout = ['hello\n']; }, /individual output lines/u],
]) {
  test(`rejects ${name} before creating execution directories`, () => {
    withCorpus(({ root, tempRoot, run }) => {
      const directory = addCase(root);
      const filename = path.join(directory, 'case.json');
      const manifest = JSON.parse(readFileSync(filename, 'utf8'));
      mutate(manifest);
      writeFileSync(filename, JSON.stringify(manifest));
      assert.throws(run, expected);
      assert.deepEqual(readdirSync(tempRoot), []);
    });
  });
}

test('rejects malformed JSON, missing license, symlink escapes, and orphan files', () => {
  withCorpus(({ root, run }) => {
    const directory = addCase(root);
    const filename = path.join(directory, 'case.json');
    const original = readFileSync(filename);
    writeFileSync(filename, '{');
    assert.throws(run, /Invalid JSON/u);
    writeFileSync(filename, original);
    const link = path.join(directory, 'files/escape.js');
    symlinkSync(filename, link);
    assert.throws(run, /no symlinks/u);
    rmSync(link);
    const license = path.join(root, 'synthetic/LICENSE');
    rmSync(license);
    assert.throws(run, /ENOENT/u);
    writeFileSync(license, '');
    assert.throws(run, /LICENSE must be nonempty/u);
    writeFileSync(license, 'Synthetic license');
    rmSync(filename);
    assert.throws(() => discoverCases(root), /Missing case.json/u);
  });
});

test('validates all metadata before running any entrypoint', () => {
  withCorpus(({ root, tempRoot, run }) => {
    addCase(root, 'a-valid', { sources: { 'entry.js': "throw Error('must not run');" } });
    addCase(root, 'z-invalid', { entryPoints: [] });
    assert.throws(run, /entryPoints/u);
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('rejects dependency trees and invalid package metadata', () => {
  withCorpus(({ root, run }) => {
    const directory = addCase(root);
    const dependencies = path.join(directory, 'files/node_modules');
    mkdirSync(dependencies);
    assert.throws(run, /External dependency trees/u);
    rmSync(dependencies, { recursive: true });
    const metadata = path.join(directory, 'files/package.json');
    writeFileSync(metadata, '{');
    assert.throws(run, /Invalid JSON/u);
    writeFileSync(metadata, '[]');
    assert.throws(run, /must be an object/u);
    writeFileSync(metadata, '{"type":"unsupported"}');
    assert.throws(run, /type must be/u);
  });
});
