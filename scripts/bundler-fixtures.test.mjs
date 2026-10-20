import assert from 'node:assert/strict';
import { chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { discoverCases, managed, runFixtures } from './bundler-fixtures.mjs';

function withCorpus(callback) {
  const managedRoot = fileURLToPath(new URL('../target/p1-test-tmp/', import.meta.url));
  return managed(managedRoot, (directory) => {
    const root = path.join(directory, 'corpus');
    const tempRoot = path.join(directory, 'runs');
    mkdirSync(path.join(root, 'synthetic'), { recursive: true });
    mkdirSync(tempRoot);
    writeFileSync(path.join(root, 'synthetic', 'LICENSE'), 'Synthetic test license, not upstream test data.');
    const driver = path.join(directory, 'driver.mjs');
    const builds = path.join(directory, 'builds.jsonl');
    writeFileSync(driver, `#!${process.execPath}
import {mkdirSync, writeFileSync, appendFileSync} from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const [root, directory, ...names] = process.argv.slice(2);
appendFileSync(${JSON.stringify(builds)}, JSON.stringify(root) + '\\n');
mkdirSync(directory, {recursive:true});
const entries = Object.fromEntries(names.map((name, i) => {
  const filename = path.join(directory, i + '.mjs');
  writeFileSync(filename, 'import ' + JSON.stringify(pathToFileURL(path.join(root, name)).href));
  return [name, filename];
}));
console.log(JSON.stringify({directory, entries, backend:'native', layout:'native-chunks-v1', warnings:[], publicationWarnings:[]}));
`);
    chmodSync(driver, 0o755);
    callback({ root, tempRoot, driver, builds, run: (options = {}) => runFixtures(root, { driver, tempRoot, ...options }) });
  });
}

function assertFailure(run, expected, options = {}) {
  const [result] = run(options);
  assert.equal(result.status, 'failed');
  assert.equal(result.stage, 'input');
  assert.match(result.error.message, expected);
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
    assert.deepEqual(run().map(({ id, status }) => [id, status]), [
      ['synthetic/a-first', 'passed'], ['synthetic/z-last', 'passed'],
    ]);
    assert.equal(readFileSync(path.join(directory, 'files/data.txt'), 'utf8'), 'hello');
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('keepOutput saves only emitted artifacts beside fixture files and removes stale outputs', () => {
  withCorpus(({ root, tempRoot, run }) => {
    const badCase = addCase(root, 'a-failure', { sources: { 'entry.js': "console.log('wrong');" } });
    const goodCase = addCase(root, 'z-success');
    const [failure, success] = run({ keepOutput: true });
    assert.equal(failure.stage, 'input');
    assert.equal(success.status, 'passed');
    assert.equal(failure.directory, undefined);
    assert(!existsSync(path.join(badCase, 'outputs')), 'invalid inputs have no bundle outputs');
    assert.equal(success.directory, path.join(goodCase, 'outputs'));
    assert.deepEqual(readdirSync(success.directory), ['0.mjs']);
    assert.match(readFileSync(path.join(success.directory, '0.mjs'), 'utf8'), /^import /u);
    assert.deepEqual(readdirSync(tempRoot), []);
    writeFileSync(path.join(success.directory, 'stale.mjs'), 'old generation');
    run({ keepOutput: true });
    assert.deepEqual(readdirSync(success.directory), ['0.mjs']);
    writeFileSync(path.join(goodCase, 'files/entry.js'), "console.log('wrong');");
    assert.equal(run({ keepOutput: true })[1].stage, 'input');
    assert(!existsSync(success.directory), 'failed reruns must not expose stale successful outputs');
    assert.throws(() => run({ keepOutput: 'true' }), /keepOutput must be a boolean/u);
  });
});

test('onResult reports each completed case before starting the next, preserving returned results', () => {
  withCorpus(({ root, run, builds, tempRoot }) => {
    addCase(root, 'a-failure', { sources: { 'entry.js': "console.log('wrong');" } });
    addCase(root, 'z-success');
    const reported = [];
    const results = run({ onResult(result) {
      reported.push(result);
      assert.equal(existsSync(builds), result.status === 'passed', 'later case must not build before reporting');
      assert.deepEqual(readdirSync(tempRoot), [], 'completed case must already be cleaned up');
    } });
    assert.deepEqual(results, reported);
    assert.deepEqual(reported.map(({ status }) => status), ['failed', 'passed']);
    assert.throws(() => run({ onResult: true }), /onResult must be a function/u);
  });
});

test('reporter errors propagate without running later cases or becoming fixture failures', () => {
  withCorpus(({ root, run, builds, tempRoot }) => {
    addCase(root, 'a-first');
    addCase(root, 'z-last');
    const failure = new Error('reporting failed');
    assert.throws(() => run({ onResult() { throw failure; } }), (error) => error === failure);
    assert.equal(readFileSync(builds, 'utf8').trim().split('\n').length, 1);
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('output directory symlinks are rejected without changing their targets', () => {
  withCorpus(({ root, tempRoot, run }) => {
    const directory = addCase(root);
    writeFileSync(path.join(tempRoot, 'untouched'), 'keep me');
    symlinkSync(tempRoot, path.join(directory, 'outputs'), 'dir');
    const [failure] = run({ keepOutput: true });
    assert.equal(failure.stage, 'output');
    assert.match(failure.error.message, /output directory \(no symlinks\)/u);
    assert.equal(readFileSync(path.join(tempRoot, 'untouched'), 'utf8'), 'keep me');
  });
});

test('generated package metadata only declares a module format, never executable payloads', () => {
  withCorpus(({ root, run, driver }) => {
    addCase(root);
    const original = readFileSync(driver, 'utf8');
    for (const [metadata, valid] of [
      [{ type: 'module' }, true],
      [{ type: 'commonjs' }, true],
      [{ type: 'unknown' }, false],
      [{ type: 'module', code: 'console.log("hello")' }, false],
    ]) {
      writeFileSync(driver, original.replace('const entries =',
        `writeFileSync(path.join(directory, 'package.json'), ${JSON.stringify(JSON.stringify(metadata))});\nconst entries =`));
      const [result] = run();
      assert.equal(result.status, valid ? 'passed' : 'failed');
      if (!valid) {
        assert.equal(result.stage, 'build');
        assert.match(result.error.message, /generated package.json/u);
      }
    }
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
    assert.deepEqual(run().map(({ status }) => status), ['passed', 'passed']);
  });
});

for (const [name, source, expected] of [
  ['output mismatch', "console.log('wrong');", /stdout mismatch/u],
  ['unexpected blank line', "console.log('hello\\n');", /stdout mismatch/u],
  ['nonzero exit', 'process.exit(7);', /exit 7/u],
  ['exit zero before completion', 'process.exit(0);', /observer did not complete/u],
  ['stderr', "console.error('warning'); console.log('hello');", /unexpected stderr/u],
  ['timeout', 'while (true) {}', /ETIMEDOUT/u],
  ['output limit', "process.stdout.write('x'.repeat(2 * 1024 * 1024));", /ENOBUFS/u],
  ['signal', "process.kill(process.pid, 'SIGKILL');", /abnormal termination/u],
  ['missing external dependency', "import 'not-a-real-fixture-package';", /unexpected runtime failure/u],
]) {
  test(`${name} fails and cleans the copied case`, () => {
    withCorpus(({ root, tempRoot, run }) => {
      addCase(root, 'failure', { sources: { 'entry.js': source } });
      assertFailure(run, expected, { timeoutMs: name === 'timeout' ? 100 : 5000 });
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
    assert.match(success.stdout, /PASS synthetic\/silent backend=native/u);
    assert.match(success.stdout, /Fixtures: 1 passed \(1 native, 0 legacy\), 0 failed, 0 unsupported, 1 emitted executions\./u);
    assert.match(success.stdout, /Bundler checks: 1 same-runtime identity case; 4 diagnostic cases\./u);
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
  ['null upstream', (m) => { m.upstream = null; }, /upstream must be an object/u],
  ['unknown expectation', (m) => { m.expectations.stderr = []; }, /exactly/u],
  ['unsupported backend', (m) => { m.expectations.backend = 'legacy'; }, /backend must be native/u],
  ['null errors', (m) => { m.expectations.errors = null; }, /must be an object/u],
  ['empty errors', (m) => { m.expectations.errors = {}; }, /must not be empty/u],
  ['unknown error entry', (m) => { m.expectations.errors = { 'absent.js': { name: 'Error' } }; }, /exactly/u],
  ['missing error name', (m) => { m.expectations.errors = { 'entry.js': { code: 'FAIL' } }; }, /exactly/u],
  ['empty error name', (m) => { m.expectations.errors = { 'entry.js': { name: '' } }; }, /nonempty text/u],
  ['nonstring error code', (m) => { m.expectations.errors = { 'entry.js': { name: 'Error', code: 1 } }; }, /nonempty text/u],
  ['unstable error message', (m) => { m.expectations.errors = { 'entry.js': { name: 'Error', message: 'path' } }; }, /exactly/u],
  ['unknown fixture option', (m) => { m.fixtureConfig = { externals: ['node:fs'] }; }, /exactly/u],
  ['non-array conditions', (m) => { m.fixtureConfig = { resolveConditions: 'source' }; }, /must be an array/u],
  ['invalid condition', (m) => { m.fixtureConfig = { resolveConditions: [1] }; }, /nonempty text/u],
  ['empty condition', (m) => { m.fixtureConfig = { resolveConditions: [' '] }; }, /nonempty text/u],
  ['import mode condition', (m) => { m.fixtureConfig = { resolveConditions: ['import'] }; }, /request-mode conditions/u],
  ['require mode condition', (m) => { m.fixtureConfig = { resolveConditions: ['require'] }; }, /request-mode conditions/u],
  ['unknown CSS option', (m) => { m.fixtureConfig = { css: { nameMangling: 'none' } }; }, /exactly/u],
  ['unknown output option', (m) => { m.fixtureConfig = { output: { directory: 'elsewhere' } }; }, /exactly/u],
  ['null fixture config', (m) => { m.fixtureConfig = null; }, /must be an object/u],
  ['invalid loader', (m) => { m.fixtureConfig = { css: { load: 'custom' } }; }, /invalid css.load/u],
  ['invalid module suffix', (m) => { m.fixtureConfig = { css: { css_modules: 'ext:../css' } }; }, /invalid css.css_modules/u],
  ['oversized fixture config', (m) => { m.fixtureConfig = { output: { base_url: 'x'.repeat(16384) } }; }, /exceeds 16 KiB/u],
  ['observer traversal', (m) => { m.cssObservation = '../observe.mjs'; }, /relative path/u],
  ['nested observer', (m) => { m.cssObservation = 'nested/observe.mjs'; }, /sibling .mjs/u],
  ['observer missing', (m) => { m.cssObservation = 'absent.mjs'; }, /ENOENT/u],
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

test('CSS output observations validate syntax without raw CSS imports and keep backend/output failures strict', () => {
  withCorpus(({ root, run, driver, builds }) => {
    const directory = addCase(root, 'css', {
      cssObservation: 'observe.mjs',
      expectations: { stdout: ['hello'], backend: 'native' },
      sources: { 'entry.js': "import './style.css';", 'style.css': '.hello { color: red; }' },
    });
    const observer = path.join(directory, 'observe.mjs');
    writeFileSync(observer, "export function observe({entry, entries}) { if (!entries[entry]) throw Error('missing'); console.log('hello'); }");
    assert.equal(run()[0].status, 'passed', 'raw Node cannot import the CSS source');
    writeFileSync(observer, "export function observe() { console.log('wrong'); }");
    const [output] = run();
    assert.equal(output.stage, 'output');
    assert.match(output.error.message, /CSS output stdout mismatch/u);
    writeFileSync(observer, 'export const = ;');
    const buildCount = readFileSync(builds, 'utf8');
    assert.equal(run()[0].stage, 'input');
    assert.equal(readFileSync(builds, 'utf8'), buildCount, 'invalid observers must not build');
    writeFileSync(observer, 'export function observe() {}');
    writeFileSync(path.join(directory, 'files/entry.js'), 'const = ;');
    assert.equal(run()[0].stage, 'input');
    assert.equal(readFileSync(builds, 'utf8'), buildCount, 'invalid CSS-case JavaScript must not build');
    writeFileSync(path.join(directory, 'files/entry.js'), "import './style.css';");
    writeFileSync(driver, readFileSync(driver, 'utf8')
      .replace("backend:'native'", "backend:'legacy'")
      .replace("layout:'native-chunks-v1'", "layout:'exact-root-set-chunks-v1'"));
    assert.equal(run()[0].stage, 'backend');
    const manifestFile = path.join(directory, 'case.json');
    const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'));
    delete manifest.expectations.backend;
    writeFileSync(manifestFile, JSON.stringify(manifest));
    assert.throws(run, /CSS output observations require native/u);
  });
});

test('custom CSS script suffixes skip CSS syntax checks but still reject malformed JavaScript', () => {
  withCorpus(({ root, run, builds }) => {
    for (const extension of ['js', 'mjs', 'cjs']) {
      const suffix = `.module.${extension}`;
      const directory = addCase(root, extension, {
        fixtureConfig: { css: { css_modules: `ext:${suffix}` } },
        cssObservation: 'observe.mjs',
        expectations: { stdout: ['hello'], backend: 'native' },
        sources: {
          'entry.js': `import './nested/style${suffix}';`,
          [`nested/style${suffix}`]: '.hello { color: red; }',
          [`nested/actual.${extension}`]: 'const valid = true;',
        },
      });
      writeFileSync(path.join(directory, 'observe.mjs'),
        "export function observe() { console.log('hello'); }");
    }
    assert.deepEqual(run().map(({ status }) => status), ['passed', 'passed', 'passed']);
    const buildCount = readFileSync(builds, 'utf8');
    for (const extension of ['js', 'mjs', 'cjs']) {
      writeFileSync(path.join(root, `synthetic/${extension}/files/nested/actual.${extension}`), 'const = ;');
    }
    for (const result of run()) {
      assert.equal(result.status, 'failed');
      assert.equal(result.stage, 'input');
      assert.match(result.error.message, /source syntax validation \(actual\.(?:js|mjs|cjs)\)/u);
    }
    assert.equal(readFileSync(builds, 'utf8'), buildCount, 'invalid JavaScript must not build');
  });
});

test('resource inventory permits only declared nonexecutable bytes', () => {
  withCorpus(({ root, run, driver }) => {
    addCase(root);
    const source = readFileSync(driver, 'utf8');
    for (const [name, declared, valid] of [
      ['style.css', true, true], ['image.svg', true, true],
      ['style.css', false, false], ['hidden.js', true, false],
    ]) {
      writeFileSync(driver, source.replace('const entries =',
        `writeFileSync(path.join(directory, ${JSON.stringify(name)}), 'resource bytes');\nconst entries =`)
        .replace("backend:'native'", `resources:${JSON.stringify(declared ? [name] : [])}, backend:'native'`));
      const [result] = run();
      assert.equal(result.status, valid ? 'passed' : 'failed');
      if (!valid) assert.equal(result.stage, 'build');
    }
  });
});

test('authored cases omit provenance and licenses; native backend requirements are enforced', () => {
  withCorpus(({ root, run }) => {
    const directory = addCase(root);
    const filename = path.join(directory, 'case.json');
    const manifest = JSON.parse(readFileSync(filename, 'utf8'));
    delete manifest.upstream;
    manifest.expectations.backend = 'native';
    writeFileSync(filename, JSON.stringify(manifest));
    rmSync(path.join(root, 'synthetic/LICENSE'));
    assert.deepEqual(run()[0].stdout, ['hello']);
  });
});

test('expected throws and async rejections preserve earlier stdout across isolated entry processes', () => {
  withCorpus(({ root, run }) => {
    addCase(root, 'errors', {
      entryPoints: ['entry.js', 'rejection.js', 'success.js'],
      expectations: {
        stdout: ['before throw', 'before rejection', 'done'],
        errors: { 'entry.js': { name: 'TypeError', code: 'LOCAL_FAIL' }, 'rejection.js': { name: 'RangeError' } },
      },
      sources: {
        'entry.js': "console.log('before throw'); throw Object.assign(new TypeError('unstable path'), {code:'LOCAL_FAIL'});",
        'rejection.js': "console.log('before rejection'); await Promise.reject(new RangeError('unstable stack'));",
        'success.js': "console.log('done');",
      },
    });
    assert.deepEqual(run()[0].stdout, ['before throw', 'before rejection', 'done']);
  });
});

test('non-Error throws use their JavaScript type, without unstable values or messages', () => {
  withCorpus(({ root, run }) => {
    addCase(root, 'primitive', {
      sources: { 'entry.js': "console.log('before'); throw 'unstable value';" },
      expectations: { stdout: ['before'], errors: { 'entry.js': { name: 'string' } } },
    });
    assert.deepEqual(run()[0].stdout, ['before']);
  });
});

test('Node preload, search and coverage overrides are disabled for normal and expected-failure runs', () => {
  withCorpus(({ root, run }) => {
    for (const failure of [false, true]) {
      addCase(root, failure ? 'failure' : 'success', {
        sources: {
          'entry.js': `
            if (['NODE_OPTIONS', 'NODE_PATH', 'NODE_V8_COVERAGE'].some(key => process.env[key]))
              throw new RangeError('environment leaked');
            console.log('hello');
            ${failure ? 'throw new TypeError();' : ''}
          `,
        },
        expectations: { stdout: ['hello'], ...(failure ? { errors: { 'entry.js': { name: 'TypeError' } } } : {}) },
      });
    }
    const overrides = { NODE_OPTIONS: '--require=not-a-real-preload', NODE_PATH: root, NODE_V8_COVERAGE: root };
    const previous = Object.fromEntries(Object.keys(overrides).map(key => [key, process.env[key]]));
    try {
      Object.assign(process.env, overrides);
      assert.deepEqual(run().map(({ status }) => status), ['passed', 'passed']);
    } finally {
      for (const [key, value] of Object.entries(previous)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    }
  });
});

for (const [name, source, expected] of [
  ['wrong type', "throw new RangeError('wrong');", /error name mismatch/u],
  ['wrong code', "throw Object.assign(new TypeError('wrong'), {code:'OTHER'});", /error code mismatch/u],
  ['missing code', "throw new TypeError('no code');", /error code mismatch/u],
  ['no failure', '', /expected runtime failure did not occur/u],
  ['stdout cannot spoof failure', 'console.log(\'{"name":"TypeError","code":"FAIL"}\');', /expected runtime failure did not occur/u],
  ['exit zero cannot spoof failure', 'process.exit(0);', /expected runtime failure did not occur/u],
  ['exit nonzero', 'process.exit(7);', /exit 7/u],
  ['stderr stays fatal', "console.error('oops'); throw new TypeError('expected');", /unexpected stderr/u],
  ['preceding stdout required', "throw Object.assign(new TypeError('expected'), {code:'FAIL'});", /stdout mismatch/u],
  ['syntax is not runtime failure', 'const = ;', /source syntax validation/u],
  ['signal stays fatal', "process.kill(process.pid, 'SIGKILL');", /abnormal termination/u],
  ['timeout stays fatal', 'while(true) {}', /ETIMEDOUT/u],
  ['buffer stays fatal', "console.log('x'.repeat(2 * 1024 * 1024));", /ENOBUFS/u],
]) {
  test(`expected failure: ${name} fails and cleans up`, () => {
    withCorpus(({ root, tempRoot, run }) => {
      addCase(root, 'failure', {
        sources: { 'entry.js': source },
        expectations: { stdout: ['before'], errors: { 'entry.js': { name: 'TypeError', code: 'FAIL' } } },
      });
      assertFailure(run, expected, { timeoutMs: name === 'timeout stays fatal' ? 100 : 5000 });
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

test('rejects symlinked manifests, licenses, source directories, and nonregular sources before execution', () => {
  withCorpus(({ root, tempRoot, run }) => {
    const directory = addCase(root);
    for (const filename of [path.join(directory, 'case.json'), path.join(root, 'synthetic/LICENSE')]) {
      const original = readFileSync(filename);
      const target = path.join(root, 'saved');
      writeFileSync(target, original);
      rmSync(filename);
      symlinkSync(target, filename);
      assert.throws(run, /symlinks/iu);
      rmSync(filename);
      writeFileSync(filename, original);
      rmSync(target);
    }
    const nested = path.join(directory, 'files/linked');
    symlinkSync(root, nested, 'dir');
    assert.throws(run, /no symlinks/u);
    rmSync(nested);
    const fifo = path.join(directory, 'files/pipe');
    const result = spawnSync('mkfifo', [fifo], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.throws(run, /regular file/u);
    assert.deepEqual(readdirSync(tempRoot), []);
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

for (const [name, source, expectations, expected] of [
  ['stdout', "console.log('wrong');", { stdout: ['hello'] }, /stdout mismatch/u],
  ['error', "throw new RangeError();", { stdout: [], errors: { 'entry.js': { name: 'TypeError' } } }, /error name mismatch/u],
  ['missing-error', '', { stdout: [], errors: { 'entry.js': { name: 'TypeError' } } }, /expected runtime failure did not occur/u],
  ['stderr', "console.error('warning'); console.log('hello');", { stdout: ['hello'] }, /unexpected stderr/u],
  ['timeout', 'while(true) {}', { stdout: [] }, /ETIMEDOUT/u],
]) {
  test(`input ${name} failure skips its build and continues to the next case`, () => {
    withCorpus(({ root, run, builds, tempRoot }) => {
      addCase(root, 'a-bad', { sources: { 'entry.js': source }, expectations });
      addCase(root, 'z-valid');
      const [failure, pass] = run({ timeoutMs: name === 'timeout' ? 1000 : 5000 });
      assert.equal(failure.id, 'synthetic/a-bad');
      assert.equal(failure.status, 'failed');
      assert.equal(failure.stage, 'input');
      assert.match(failure.error.message, expected);
      assert.equal(pass.status, 'passed');
      assert.equal(pass.id, 'synthetic/z-valid');
      assert.equal(readFileSync(builds, 'utf8').trim().split('\n').length, 1, 'invalid input must not build');
      assert.deepEqual(readdirSync(tempRoot), []);
    });
  });
}

test('native requirements use the driver backend, not successful runtime observations', () => {
  withCorpus(({ root, run, driver }) => {
    addCase(root, 'required', { expectations: { stdout: ['hello'], backend: 'native' } });
    addCase(root, 'unmarked');
    writeFileSync(driver, readFileSync(driver, 'utf8')
      .replace("backend:'native'", "backend:'legacy'")
      .replace("layout:'native-chunks-v1'", "layout:'exact-root-set-chunks-v1'"));
    const [failure, pass] = run();
    assert.equal(failure.stage, 'backend');
    assert.equal(failure.status, 'failed');
    assert.equal(failure.backend, 'legacy');
    assert.match(failure.error.message, /native backend required/u);
    assert.equal(pass.status, 'passed');
    assert.equal(pass.backend, 'legacy');
  });
});

test('planning diagnostics remain visible and never admit unrelated build warnings', () => {
  withCorpus(({ root, run, driver }) => {
    addCase(root);
    const source = readFileSync(driver, 'utf8');
    const warning = 'effect-order cycle; synthetic';
    writeFileSync(driver, source.replace('warnings:[]',
      `warnings:[${JSON.stringify(warning)}], planWarnings:[${JSON.stringify(warning)}]`));
    const [pass] = run();
    assert.equal(pass.status, 'passed');
    assert.deepEqual(pass.warnings, [warning]);
    for (const [warnings, planWarnings, expected] of [
      [['cache write: failed'], [], /cache write warnings/u],
      [['unknown'], ['unknown'], /unknown planning warnings/u],
      [[warning, 'cache write: failed'], [warning], /cache write warnings/u],
    ]) {
      writeFileSync(driver, source.replace('warnings:[]',
        `warnings:${JSON.stringify(warnings)}, planWarnings:${JSON.stringify(planWarnings)}`));
      const [failure] = run();
      assert.equal(failure.stage, 'build');
      assert.match(failure.error.message, expected);
    }
  });
});

test('CLI reports input/build/output failures and native CJS, honoring forced color and NO_COLOR', () => {
  withCorpus(({ root }) => {
    addCase(root, 'a-input', { sources: { 'entry.js': "console.log('wrong');" } });
    addCase(root, 'b-build', {
      sources: { 'entry.js': "const name='./dep.js'; await import(name); console.log('hello');", 'dep.js': '' },
    });
    addCase(root, 'd-output', {
      sources: { 'entry.js': "console.log(process.argv[1].includes('.bundler-generation-') ? 'wrong' : 'hello');" },
    });
    addCase(root, 'y-cjs', {
      sources: { 'entry.js': "import './dep.cjs'; console.log('hello');", 'dep.cjs': 'module.exports = 1;' },
      expectations: { stdout: ['hello'], backend: 'native' },
    });
    addCase(root, 'z-valid');
    const runner = fileURLToPath(new URL('./bundler-fixtures.mjs', import.meta.url));
    const env = { ...process.env, FORCE_COLOR: '1' };
    delete env.NO_COLOR;
    const colored = spawnSync(process.execPath, [runner, root], { encoding: 'utf8', env });
    assert.equal(colored.status, 1, colored.stderr);
    assert(colored.stdout.includes('\x1b[32mPASS\x1b[0m synthetic/y-cjs backend=native\n'));
    assert(colored.stdout.includes('\x1b[32mPASS\x1b[0m synthetic/z-valid backend=native\n'));
    assert(!colored.stdout.includes('\x1b[31mbackend=native'));
    const result = spawnSync(process.execPath, [runner, root], {
      encoding: 'utf8', env: { ...env, NO_COLOR: '1' },
    });
    assert.equal(result.status, 1, result.stderr);
    assert(!result.stdout.includes('\x1b['));
    assert.equal(colored.stdout.replace(/\x1b\[\d+m/gu, ''), result.stdout);
    for (const [stage, name] of [['INPUT', 'a-input'], ['BUILD', 'b-build'], ['OUTPUT', 'd-output']]) {
      assert.match(result.stdout, new RegExp(`FAIL ${stage} synthetic/${name}`, 'u'));
    }
    assert.match(result.stdout, /PASS synthetic\/z-valid backend=native/u);
    assert.match(result.stdout, /2 passed \(2 native, 0 legacy\), 3 failed, 0 unsupported/u);
    assert.match(result.stderr, /public build pipeline.*exit 1/su);
    assert.match(result.stdout, /Bundler checks: 1 same-runtime identity case; 4 diagnostic cases/u);
  });
});

test('CLI labels malformed corpus as input metadata before any execution', () => {
  withCorpus(({ root, tempRoot }) => {
    addCase(root, 'a-valid', {
      sources: { 'entry.js': `import {writeFileSync} from 'node:fs'; writeFileSync(${JSON.stringify(path.join(tempRoot, 'executed'))}, 'bad');` },
    });
    addCase(root, 'z-invalid', { entryPoints: [] });
    const runner = fileURLToPath(new URL('./bundler-fixtures.mjs', import.meta.url));
    const result = spawnSync(process.execPath, [runner, root], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.match(result.stderr, /INPUT metadata:.*entryPoints/su);
    assert.match(result.stderr, /Caused by:/u);
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('CLI prints completed results while the next case is still executing', () => {
  withCorpus(({ root, tempRoot }) => {
    const marker = path.join(tempRoot, 'first-result-received');
    addCase(root, 'a-first');
    addCase(root, 'z-wait', { sources: { 'entry.js': `
      import { existsSync } from 'node:fs';
      while (!existsSync(${JSON.stringify(marker)})) {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      console.log('hello');
    ` } });
    const runner = fileURLToPath(new URL('./bundler-fixtures.mjs', import.meta.url));
    const observer = `
      import { spawn } from 'node:child_process';
      import { writeFileSync } from 'node:fs';
      const child = spawn(process.execPath, ${JSON.stringify([runner, root])}, {
        env: { ...process.env, NO_COLOR: '1' },
      });
      let stdout = '';
      child.stdout.on('data', chunk => {
        stdout += chunk;
        process.stdout.write(chunk);
        if (stdout.includes('PASS synthetic/a-first backend=native\\n')) {
          writeFileSync(${JSON.stringify(marker)}, 'received');
        }
      });
      child.stderr.pipe(process.stderr);
      child.on('error', error => { throw error; });
      child.on('close', code => { process.exitCode = code ?? 1; });
    `;
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', observer], { encoding: 'utf8' });
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stdout, /PASS synthetic\/a-first backend=native/u);
    // The marker lets source execution finish; its node:fs import then fails the build.
    assert.match(result.stdout, /FAIL BUILD synthetic\/z-wait/u);
    assert.doesNotMatch(result.stderr, /ETIMEDOUT/u);
  });
});

test('CLI keep-output flag prints exact adjacent bundle paths without retaining workspaces', () => {
  withCorpus(({ root }) => {
    const badCase = addCase(root, 'a-failure', { sources: { 'entry.js': "console.log('wrong');" } });
    const goodCase = addCase(root, 'z-success');
    const runner = fileURLToPath(new URL('./bundler-fixtures.mjs', import.meta.url));
    for (const args of [[runner, '--keep-output', root], [runner, root, '--keep-output']]) {
      const result = spawnSync(process.execPath, args, {
        encoding: 'utf8', env: { ...process.env, NO_COLOR: '1' },
      });
      const outputs = [...result.stdout.matchAll(/^OUTPUT ([^:]+): (.+)$/gmu)];
      assert.equal(result.status, 1, result.stderr);
      assert.deepEqual(outputs.map((match) => [match[1], match[2]]),
        [['synthetic/z-success', path.join(goodCase, 'outputs')]]);
      assert(!existsSync(path.join(badCase, 'outputs')));
      assert.deepEqual(readdirSync(path.join(goodCase, 'outputs')), ['entry.js', 'package.json']);
    }
    const invalid = spawnSync(process.execPath, [runner, '--keep-output', '--unknown', root], { encoding: 'utf8' });
    assert.equal(invalid.status, 1);
    assert.equal(invalid.stdout, '');
    assert.match(invalid.stderr, /Usage:/u);
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
