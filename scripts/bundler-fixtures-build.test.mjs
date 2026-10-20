import assert from 'node:assert/strict';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { buildProject, execute, managed, observeEntries, runBundlerCases, runFixtures } from './bundler-fixtures.mjs';

const project = fileURLToPath(new URL('../', import.meta.url));
const managedRoot = path.join(project, 'target/p1-test-tmp');

function withCorpus(source, callback, extra = {}) {
  mkdirSync(managedRoot, { recursive: true });
  const directory = mkdtempSync(path.join(managedRoot, 'differential-test-'));
  const root = path.join(directory, 'corpus');
  const caseDir = path.join(root, 'synthetic/example');
  const tempRoot = path.join(directory, 'runs');
  mkdirSync(path.join(caseDir, 'files'), { recursive: true });
  mkdirSync(tempRoot);
  writeFileSync(path.join(caseDir, 'case.json'), JSON.stringify({
    id: 'synthetic/example',
    moduleType: 'module', entryPoints: ['entry.js'],
    expectations: { stdout: ['hello'] }, goals: ['Exercise the differential harness.'],
  }));
  writeFileSync(path.join(caseDir, 'files/entry.js'), source);
  for (const [name, text] of Object.entries(extra)) writeFileSync(path.join(caseDir, 'files', name), text);
  try {
    callback({
      run: (options = {}) => runFixtures(root, { tempRoot, ...options }), tempRoot, caseDir,
      configure: (changes) => {
        const filename = path.join(caseDir, 'case.json');
        writeFileSync(filename, JSON.stringify({ ...JSON.parse(readFileSync(filename, 'utf8')), ...changes }));
      },
    });
    assert.deepEqual(readdirSync(tempRoot), [], 'managed runs leaked');
    assert.equal(readFileSync(path.join(caseDir, 'files/entry.js'), 'utf8'), source);
  } finally { rmSync(directory, { recursive: true }); }
}

function assertFailure(run, stage, expected, options = {}) {
  const [result] = run(options);
  assert.equal(result.status, 'failed');
  assert.equal(result.stage, stage);
  assert.match(result.error.message, expected);
  return result;
}

test('real public pipeline executes emitted output without modifying original fixtures', () => {
  withCorpus("console.log('hello');", ({ run, configure }) => {
    for (const workers of [1, 2, 4, 6]) {
      configure({ fixtureConfig: { workers } });
      assert.deepEqual(run(), [{ id: 'synthetic/example', status: 'passed', backend: 'native', executions: 1, stdout: ['hello'] }]);
    }
  });
});

for (const [name, relocated] of [
  ['barrels', []],
  ['bridge', ['x.mjs', 'y.mjs', 'z.mjs']],
  ['diamond', ['hub.mjs', 'left.mjs', 'right.mjs']],
  ['failure-cache', ['x.mjs', 'y.mjs']],
  ['helper-names', ['x.mjs', 'y.mjs']],
  ['native-fallback', []],
  ['write-restrictions', ['state.mjs', 'writer.mjs']],
]) {
  test(`adversarial library ${name} proves placement or explicit native fallback`, () => {
    const fixture = path.join(project, 'fixtures/bundler/authored', `adversarial-library-${name}`);
    const manifest = JSON.parse(readFileSync(path.join(fixture, 'case.json'), 'utf8'));
    managed(managedRoot, directory => {
      const root = path.join(directory, 'source');
      cpSync(path.join(fixture, 'files'), root, {recursive: true});
      const members = [0, manifest.fixtureConfig.output.min_chunk_bytes].map(min_chunk_bytes => {
        const build = buildProject(path.join(project, 'target/debug/examples/build'),
          root, manifest.entryPoints, 30000, {
            ...manifest.fixtureConfig,
            output: {...manifest.fixtureConfig.output, min_chunk_bytes},
          });
        assert.equal(build.backend, 'native');
        return new Map(build.segmentMembers.map(member =>
          [`${path.relative(root, member.source)}:${member.segment}`, member.artifact]));
      });
      assert.deepEqual([...members[0].keys()], [...members[1].keys()], 'placement must preserve retained segments');
      if (relocated.length === 0) {
        assert.deepEqual(members[0], members[1], 'unsupported islands must retain native owners');
      } else {
        for (const source of relocated) {
          assert([...members[0]].some(([key, owner]) =>
            key.startsWith(`${source}:`) && members[1].get(key) !== owner),
          `${source} must have a genuinely relocated segment`);
        }
      }
    });
  });
}

for (const [name, source, assertions] of [
  ['guarded arguments and spread', `
    function helper(...values) {
      globalThis.events.push('body');
      return [this === undefined, values];
    }
    export async function run(enabled) {
      globalThis.events = [];
      const value = enabled && helper(
        (globalThis.events.push('argument'), 1),
        ...{ *[Symbol.iterator]() { globalThis.events.push('spread'); yield 2; } }
      );
      return [await value, globalThis.events];
    }`,
  `assert.deepEqual(await ns.run(false), [false, []]);
   assert.deepEqual(await ns.run(true), [[true, [1, 2]], ['argument', 'spread', 'body']]);`],
  ['throw identity and finally', `
    function fail(value) { globalThis.events.push('body'); throw value; }
    export async function run(value) {
      globalThis.events = [];
      try { fail(value); }
      catch (error) { globalThis.events.push(error === value ? 'caught' : 'wrong'); }
      finally { globalThis.events.push('finally'); }
      return globalThis.events;
    }`,
  `for (const value of [undefined, null, {reason: 'identity'}, new ReferenceError('user')])
     assert.deepEqual(await ns.run(value), ['body', 'caught', 'finally']);`],
  ['recursive binding identity', `
    function even(n) { return n ? odd(n - 1) : true; }
    function odd(n) { return n ? even(n - 1) : false; }
    export async function run() { return {values: [even(6), odd(5)], even}; }`,
  `const a = await ns.run(), b = await ns.run();
   assert.deepEqual(a.values, [true, true]);
   assert.equal(a.even, b.even);
   assert.equal(a.even(4), true);`],
]) {
  test(`relocation option matrix preserves ${name}`, () => {
    withCorpus(source, ({ caseDir }) => {
      const root = path.join(caseDir, 'files');
      const observer = path.join(caseDir, 'observe.mjs');
      writeFileSync(observer, `
        import assert from 'node:assert/strict';
        import {pathToFileURL} from 'node:url';
        export async function observe({entries, entry}) {
          const ns = await import(pathToFileURL(entries[entry]));
          ${assertions}
          console.log('hello');
        }`);
      const manifest = {
        id: `matrix/${name}`, entryPoints: ['entry.js'],
        expectations: {stdout: ['hello']},
      };
      const observerEntries = {'entry.js': observer};
      const baseline = observeEntries(manifest, observerEntries, root, 5000, 'source', {
        entries: {'entry.js': path.join(root, 'entry.js')}, directory: root, mode: 'source',
      });
      for (const synthesize_async_boundaries of [false, true]) {
        for (const min_chunk_bytes of [0, 4096]) {
          const build = buildProject(path.join(project, 'target/debug/examples/build'),
            root, manifest.entryPoints, 30000, {
              output: {synthesize_async_boundaries, min_chunk_bytes},
            });
          assert.equal(build.generatedBoundaries > 0, synthesize_async_boundaries,
            'enabled extraction must run; disabled extraction must not create boundaries');
          const entryArtifact = path.relative(build.directory, build.entries['entry.js']);
          if (synthesize_async_boundaries) {
            assert.equal(build.segmentMembers.some(member => member.artifact !== entryArtifact),
              min_chunk_bytes === 0,
              'typed members must prove separate loading versus co-location');
          }
          const result = observeEntries(manifest, observerEntries, root, 5000,
            `synthesis=${synthesize_async_boundaries}, size=${min_chunk_bytes}`,
            {...build, mode: 'bundled'});
          assert.deepEqual(result, baseline);
        }
      }
    }, {'package.json': '{"type":"module"}'});
  });
}
test('explicit conditions select package exports while JSONC tsconfigs stay inert', () => {
  managed(managedRoot, (root) => {
    writeFileSync(path.join(root, 'entry.js'), "import {value} from 'choice'; console.log(value);");
    writeFileSync(path.join(root, 'package.json'), '{"type":"module"}');
    writeFileSync(path.join(root, 'tsconfig.json'), `{
      // Neither the driver nor the Node observer ingests compiler conditions.
      "extends": "./missing.json",
      "references": [{"path": "./missing-project"},],
      "compilerOptions": {"customConditions": ["source"], "customSourceConditions": ["source"],},
    }`);
    const pkg = path.join(root, 'node_modules/choice');
    mkdirSync(pkg, { recursive: true });
    writeFileSync(path.join(pkg, 'package.json'), JSON.stringify({
      type: 'module',
      exports: { '.': { source: './source.js', default: './default.js' } },
    }));
    writeFileSync(path.join(pkg, 'source.js'), "export const value = 'source';");
    writeFileSync(path.join(pkg, 'default.js'), "export const value = 'hello';");
    const driver = path.join(project, `target/debug/examples/build${process.platform === 'win32' ? '.exe' : ''}`);
    for (const [resolveConditions, value] of [[[], 'hello'], [['source'], 'source']]) {
      const manifest = {
        id: 'synthetic/example', entryPoints: ['entry.js'],
        fixtureConfig: { resolveConditions }, expectations: { stdout: [value] },
      };
      const baseline = observeEntries(manifest, { 'entry.js': path.join(root, 'entry.js') }, root);
      const build = buildProject(driver, root, manifest.entryPoints, 30000, manifest.fixtureConfig);
      assert.deepEqual(observeEntries(manifest, build.entries, build.directory), baseline);
    }
  });
});

for (const suffix of ['.module.css', '.module.cjs']) {
  test(`explicit fixture configuration selects ${suffix} CSS transforms and preserves byte resources`, () => {
    withCorpus(`export {buttonLabel} from './style${suffix}';`, ({ run, configure, caseDir }) => {
      configure({
        fixtureConfig: { output: { base_url: '/static/' }, css: { css_modules: suffix === '.module.css' ? true : `ext:${suffix}` } },
        cssObservation: 'observe.mjs',
        expectations: { stdout: ['CSS mapping'], backend: 'native' },
      });
      const observer = path.join(caseDir, 'observe.mjs');
      writeFileSync(observer, `
      import assert from 'node:assert/strict';
      import {readFileSync} from 'node:fs';
      import path from 'node:path';
      import {pathToFileURL} from 'node:url';
      export async function observe({entries, entry, directory, stylesheets}) {
        const {buttonLabel} = await import(pathToFileURL(entries[entry]));
        assert.match(buttonLabel, /^_[A-Za-z0-9_-]{10}$/u);
        assert(readFileSync(path.join(directory, Object.values(stylesheets)[0]), 'utf8').includes('.' + buttonLabel));
        console.log('CSS mapping');
      }
    `);
      assert.equal(run()[0].status, 'passed');
      writeFileSync(observer, "export function observe() { throw Error('output must fail'); }");
      assertFailure(run, 'output', /unexpected runtime failure/u);
    }, { [`style${suffix}`]: '.button-label { color: red; }' });
  });
}

test('driver rejects unknown, malformed and oversized config before publication', () => {
  withCorpus("console.log('hello');", ({ caseDir }) => {
    const root = path.join(caseDir, 'files');
    const driver = path.join(project, 'target/debug/examples/build');
    const configFile = path.join(root, '.bundler-fixture-config.json');
    const output = path.join(root, '.bundler-output');
    for (const config of [
      '{"css":{"nameMangling":"none"}}',
      '{"output":{"directory":"other"}}',
      '{"output":{"min_chunk_bytes":-1}}',
      '{"output":{"min_chunk_bytes":1.5}}',
      '{"entries":[]}', '{"css":{"load":"plugin"}}',
      '{"css":{"css_modules":"ext:../css"}}',
      '{"output":{"base_url":"javascript:bad"}}', '{',
      '{"resolveConditions":"source"}', '{"resolveConditions":[1]}',
      '{"resolveConditions":[" "]}',
      '{"resolveConditions":["import"]}', '{"resolveConditions":["require"]}',
      ...[0, -1, 1.5, 65, '2', null].map(workers => JSON.stringify({ workers })),
      JSON.stringify({ output: { base_url: 'x'.repeat(16384) } }),
    ]) {
      writeFileSync(configFile, config);
      const result = execute(driver, [root, output, '--fixture-config', configFile, 'entry.js'], root, 30000);
      assert.equal(result.status, 1, config);
      assert(result.stderr, 'configuration rejection must explain failure');
      assert(!existsSync(path.join(output, '.bundler-current.json')), 'invalid config published');
    }
    const missing = execute(driver, [root, output, '--fixture-config'], root);
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /missing fixture config file/u);
  });
});

test('retained outputs are exact executable artifacts, including on runtime failure', () => {
  withCorpus("console.log('hello');", ({ run, caseDir }) => {
    const [pass] = run({ keepOutput: true });
    assert.equal(pass.directory, path.join(caseDir, 'outputs'));
    assert.deepEqual(readdirSync(pass.directory), ['entry.js', 'package.json']);
    const observed = execute(process.execPath, [path.join(pass.directory, 'entry.js')], pass.directory);
    assert.equal(observed.status, 0, observed.stderr);
    assert.equal(observed.stdout, 'hello\n');
  });
  const source = "console.log(process.argv[1].includes('.bundler-generation-') ? 'wrong' : 'hello');";
  withCorpus(source, ({ run, caseDir }) => {
    const failure = assertFailure(run, 'output', /bundled stdout mismatch/u, { keepOutput: true });
    assert.equal(failure.directory, path.join(caseDir, 'outputs'));
    assert.deepEqual(readdirSync(failure.directory), ['entry.js', 'package.json']);
    assert(readFileSync(path.join(failure.directory, 'entry.js'), 'utf8').includes(source));
  });
});

test('unsupported capabilities are explicitly reported and never counted as passes', () => {
  withCorpus("void import.meta.url; console.log('hello');", ({ run }) => {
    const result = run({ unsupported: new Map([['synthetic/example', 'import.meta is outside this target']]) });
    assert.equal(result[0].status, 'unsupported');
    assert.equal(result[0].executions, 0);
    assert.match(result[0].reason, /import.meta/u);
    assert.throws(() => run({ unsupported: new Map([['unknown/case', 'not supported']]) }), /unknown unsupported/u);
  });
});

test('an undeclared unsupported build is a failure, not a skip', () => {
  withCorpus("const name = './dep.js'; await import(name); console.log('hello');", ({ run }) => {
    assertFailure(run, 'build', /public build pipeline.*exit 1/su);
  }, { 'dep.js': 'export const value = 1;' });
});

for (const [name, source, expected, options] of [
  ['native output mismatch', "console.log('wrong');", /source stdout mismatch/u, {}],
  ['native timeout', 'while(true) {}', /ETIMEDOUT/u, { timeoutMs: 100 }],
  ['generated timeout', "if(process.argv[1].includes('.bundler-generation-')) { while(true) {} } console.log('hello');", /ETIMEDOUT/u, { timeoutMs: 1000 }],
  ['generated output mismatch', "console.log(process.argv[1].includes('.bundler-generation-') ? 'wrong' : 'hello');", /bundled stdout mismatch/u, {}],
  ['generated stderr', "if(process.argv[1].includes('.bundler-generation-')) console.error('unexpected'); console.log('hello');", /unexpected stderr/u, {}],
  ['generated nonzero exit', "if(process.argv[1].includes('.bundler-generation-')) process.exit(7); console.log('hello');", /exit 7/u, {}],
  ['output bound', "console.log('x'.repeat(2 * 1024 * 1024));", /ENOBUFS/u, {}],
]) {
  test(`${name} fails and cleans managed runs`, () => {
    withCorpus(source, ({ run }) => assertFailure(run,
      name.startsWith('generated') ? 'output' : 'input', expected, options));
  });
}

test('native-required cases use the authoritative native backend for ESM and opaque CJS', () => {
  withCorpus("console.log('hello');", ({ run, configure }) => {
    configure({ expectations: { stdout: ['hello'], backend: 'native' } });
    const [result] = run();
    assert.equal(result.status, 'passed');
    assert.equal(result.backend, 'native');
  });
  withCorpus("import './dep.cjs'; console.log('hello');", ({ run, configure }) => {
    const [result] = run();
    assert.equal(result.status, 'passed');
    assert.equal(result.backend, 'native');
    configure({ expectations: { stdout: ['hello'], backend: 'native' } });
    const [required] = run();
    assert.equal(required.status, 'passed');
    assert.equal(required.backend, 'native');
  }, { 'dep.cjs': 'module.exports = 1;' });
});

test('runtime failures and preceding effects match across native and bundled multiple-entry processes', () => {
  withCorpus("console.log('hello'); throw Object.assign(new TypeError('native path'), {code:'FAIL'});",
    ({ run, configure }) => {
      configure({
        entryPoints: ['entry.js', 'async.js', 'success.js'],
        expectations: {
          stdout: ['hello', 'async', 'done'],
          errors: { 'entry.js': { name: 'TypeError', code: 'FAIL' }, 'async.js': { name: 'RangeError' } },
        },
      });
      const [result] = run();
      assert.equal(result.status, 'passed');
      assert.equal(result.backend, 'native');
      assert.equal(result.executions, 3);
    }, {
      'async.js': "console.log('async'); await Promise.reject(new RangeError('rejection'));",
      'success.js': "console.log('done');",
    });
});

test('native async cycles retain visible planning warnings', () => {
  withCorpus("import './a.js'; console.log('hello');", ({ run, configure }) => {
    configure({ expectations: { stdout: ['hello'], backend: 'native' } });
    const [result] = run();
    assert.equal(result.status, 'passed');
    assert.equal(result.backend, 'native');
    assert(result.warnings.length > 0);
    assert(result.warnings.every(warning => warning.startsWith('effect-order cycle; ')));
  }, {
    'a.js': "import './b.js'; await 0;",
    'b.js': "import './a.js'; await 0;",
  });
});

for (const [name, source, expected] of [
  ['native wrong type', "console.log('hello'); throw new RangeError('wrong');", /source.*error name mismatch/u],
  ['bundled wrong type', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) throw new RangeError('wrong'); throw Object.assign(new TypeError('right'), {code:'FAIL'});", /bundled.*error name mismatch/u],
  ['bundled wrong code', "console.log('hello'); throw Object.assign(new TypeError('error'), {code:process.argv[1].includes('.bundler-generation-') ? 'OTHER' : 'FAIL'});", /bundled.*error code mismatch/u],
  ['bundled missing code', "console.log('hello'); const error = new TypeError('error'); if(!process.argv[1].includes('.bundler-generation-')) error.code = 'FAIL'; throw error;", /bundled.*error code mismatch/u],
  ['bundled missing failure', "console.log('hello'); if(!process.argv[1].includes('.bundler-generation-')) throw Object.assign(new TypeError('right'), {code:'FAIL'});", /bundled.*expected runtime failure did not occur/u],
  ['bundled missing earlier effects', "if(!process.argv[1].includes('.bundler-generation-')) console.log('hello'); throw Object.assign(new TypeError('right'), {code:'FAIL'});", /bundled stdout mismatch/u],
  ['bundled stderr', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) console.error('warning'); throw Object.assign(new TypeError('right'), {code:'FAIL'});", /unexpected stderr/u],
  ['bundled nonzero exit', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) process.exit(7); throw Object.assign(new TypeError(), {code:'FAIL'});", /exit 7/u],
  ['bundled signal', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) process.kill(process.pid, 'SIGKILL'); throw Object.assign(new TypeError(), {code:'FAIL'});", /abnormal termination/u],
  ['bundled timeout', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) { while(true) {} } throw Object.assign(new TypeError(), {code:'FAIL'});", /ETIMEDOUT/u],
  ['bundled output limit', "console.log('hello'); if(process.argv[1].includes('.bundler-generation-')) console.log('x'.repeat(2 * 1024 * 1024)); throw Object.assign(new TypeError(), {code:'FAIL'});", /ENOBUFS/u],
  ['build failure remains failure', "console.log('hello'); const name='./dep.js'; await import(name);", /public build pipeline.*exit 1/su],
]) {
  test(`expected runtime failure: ${name} cannot pass`, () => {
    withCorpus(source, ({ run, configure }) => {
      configure({ expectations: { stdout: ['hello'], errors: { 'entry.js': { name: 'TypeError', code: 'FAIL' } } } });
      assertFailure(run, name.startsWith('native') ? 'input' : name.startsWith('build') ? 'build' : 'output',
        expected, { timeoutMs: name === 'bundled timeout' ? 1000 : 5000 });
    }, { 'dep.js': "throw Object.assign(new TypeError('right'), {code:'FAIL'});" });
  });
}

test('differential compares observed error codes even when the expectation omits code', () => {
  withCorpus("console.log('hello'); throw Object.assign(new TypeError('path-dependent message'), {code: process.argv[1].includes('.bundler-generation-') ? 'BUNDLED' : 'NATIVE'});",
    ({ run, configure }) => {
      configure({ expectations: { stdout: ['hello'], errors: { 'entry.js': { name: 'TypeError' } } } });
      assertFailure(run, 'output', /emitted\/source runtime observation mismatch/u);
    });
});

test('differential compares stdout per entry, not just concatenated lines', () => {
  withCorpus("console.log('hello'); if(!process.argv[1].includes('.bundler-generation-')) console.log('moved');",
    ({ run, configure }) => {
      configure({ entryPoints: ['entry.js', 'second.js'], expectations: { stdout: ['hello', 'moved'] } });
      assertFailure(run, 'output', /emitted\/source runtime observation mismatch/u);
    }, { 'second.js': "if(process.argv[1].includes('.bundler-generation-')) console.log('moved');" });
});

test('differential validates the whole corpus before executing source or invoking the driver', () => {
  withCorpus("throw Error('must not run');", ({ run, caseDir, tempRoot }) => {
    const invalid = path.join(path.dirname(caseDir), 'z-invalid');
    mkdirSync(invalid);
    writeFileSync(path.join(invalid, 'case.json'), '{}');
    assert.throws(() => run({ driver: 'missing-driver' }), /exactly/u);
    assert.deepEqual(readdirSync(tempRoot), []);
  });
});

test('same-runtime multi-entry identity and bundler-only diagnostics use public pipeline', () => {
  assert.deepEqual(runBundlerCases(), { identity: 1, diagnostics: 4 });
});

test('timed-out runtime child is killed and reaped', () => {
  let failure;
  try { execute(process.execPath, ['-e', 'while(true) {}'], project, 100); }
  catch (error) { failure = error; }
  assert.equal(failure?.code, 'ETIMEDOUT');
  assert(Number.isInteger(failure.childPid) && failure.childPid > 0);
  assert.throws(() => process.kill(failure.childPid, 0), { code: 'ESRCH' });
});
