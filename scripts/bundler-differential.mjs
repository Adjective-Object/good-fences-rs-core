import assert from 'node:assert/strict';
import {
  existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { copyCase, discoverCases, execute, managed, observeEntries, successful } from './bundler-fixtures.mjs';
export { execute } from './bundler-fixtures.mjs';

const repository = fileURLToPath(new URL('../', import.meta.url));
const defaultRoot = path.join(repository, 'fixtures/bundler');
const defaultDriver = path.join(repository, `target/debug/examples/build${process.platform === 'win32' ? '.exe' : ''}`);
const defaultTempRoot = path.join(repository, 'target/p1-test-tmp');

export function buildProject(driver, root, entries, timeoutMs = 30000) {
  assert(existsSync(driver), `Missing library driver; run cargo build -p bundler --example build (${driver})`);
  const result = execute(driver, [root, path.join(root, '.bundler-output'), ...entries], root, timeoutMs);
  const build = JSON.parse(successful(result, 'public build pipeline'));
  assert.deepEqual(build.warnings, [], 'cache write warnings');
  assert.deepEqual(build.publicationWarnings, [], 'publication warnings');
  assert(['native', 'legacy'].includes(build.backend), 'driver must report its selected backend');
  assert.equal(build.layout, build.backend === 'native' ? 'native-single-chunk-v1' : 'exact-root-set-chunks-v1',
    'physical layout must match the reported backend');
  for (const file of readdirSync(build.directory, { recursive: true, withFileTypes: true })) {
    if (file.isDirectory()) continue;
    assert(file.isFile(), 'generation contains a nonregular artifact');
    assert(/\.(?:mjs|cjs)$/u.test(file.name), 'generation must contain executable JavaScript, not code JSON');
    const source = readFileSync(path.join(file.parentPath, file.name), 'utf8');
    assert(!/registerHooks|experimental-loader|good-fences:|new Function|eval\(/u.test(source),
      'generated code must use the ordinary file loader');
  }
  for (const entry of entries) {
    assert.equal(typeof build.entries[entry], 'string', 'missing generated entry file');
    assert(build.entries[entry].startsWith(`${build.directory}${path.sep}`), 'entry escaped generation');
    assert(existsSync(build.entries[entry]), 'generated entry file missing');
  }
  return build;
}

export function runDifferential(root = defaultRoot, {
  driver = defaultDriver, tempRoot = defaultTempRoot, timeoutMs = 5000,
  unsupported = new Map(),
} = {}) {
  // Reuse full baseline validation, without changing its execution semantics.
  const cases = discoverCases(root);
  assert(unsupported instanceof Map, 'unsupported must map case IDs to explicit reasons');
  for (const [id, reason] of unsupported) {
    assert(cases.some(({ manifest }) => manifest.id === id), `unknown unsupported case: ${id}`);
    assert.equal(typeof reason, 'string');
    assert(reason.trim(), 'unsupported case needs a reason');
  }
  return cases.map(({ manifest, files }) => managed(tempRoot, (directory) => {
    const native = path.join(directory, 'native');
    const source = path.join(directory, 'source');
    for (const destination of [native, source]) {
      copyCase(files, destination, manifest);
    }
    const baseline = observeEntries(manifest,
      Object.fromEntries(manifest.entryPoints.map((entry) => [entry, path.join(native, entry)])),
      native, timeoutMs, 'native');
    if (unsupported.has(manifest.id)) {
      return { id: manifest.id, status: 'unsupported', reason: unsupported.get(manifest.id), executions: 0 };
    }
    const build = buildProject(driver, source, manifest.entryPoints);
    if (manifest.expectations.backend === 'native') {
      assert.equal(build.backend, 'native', `${manifest.id}: native backend required; legacy fallback selected`);
    }
    const bundled = observeEntries(manifest, build.entries, source, timeoutMs, 'bundled');
    assert.deepEqual(bundled, baseline, `${manifest.id}: emitted/native runtime observation mismatch`);
    return { id: manifest.id, status: 'passed', backend: build.backend, executions: manifest.entryPoints.length };
  }));
}

function writeSources(root, sources) {
  mkdirSync(root, { recursive: true });
  for (const [name, source] of Object.entries(sources)) {
    const filename = path.join(root, name);
    mkdirSync(path.dirname(filename), { recursive: true });
    writeFileSync(filename, source);
  }
}

export function runBundlerCases({ driver = defaultDriver, tempRoot = defaultTempRoot } = {}) {
  return managed(tempRoot, (directory) => {
    const root = path.join(directory, 'identity');
    writeSources(root, {
      'a.mjs': "import shared from './shared.cjs'; export {shared}; export default shared; export const bump = shared.bump;",
      'b.mjs': "export {default, shared, bump} from './a.mjs';",
      'shared.cjs': "console.log('init'); const shared = {count: 0}; shared.bump = () => ++shared.count; module.exports = shared;",
    });
    const observation = `
const [a,b] = await Promise.all([load('a.mjs'), load('b.mjs')]);
console.log(a.shared === b.shared, a.bump === b.bump, a.shared.count, a === await load('a.mjs'));
a.bump(); console.log(a.shared.count, b.shared.count);
console.log(staticDefault === a.shared, staticNamed === a.shared, staticBump === a.bump);
`;
    const nativeDriver = path.join(root, 'native.mjs');
    writeFileSync(nativeDriver, `import staticDefault, {shared as staticNamed, bump as staticBump} from './a.mjs'; const load = name => import('./' + name); ${observation}`);
    const baseline = successful(execute(process.execPath, [nativeDriver], root), 'same-runtime native');
    const build = buildProject(driver, root, ['a.mjs', 'b.mjs']);
    const generatedDriver = path.join(root, 'observe.mjs');
    const entries = Object.fromEntries(Object.entries(build.entries).map(([name, filename]) => [name, pathToFileURL(filename).href]));
    writeFileSync(generatedDriver, `import staticDefault, {shared as staticNamed, bump as staticBump} from ${JSON.stringify(entries['a.mjs'])}; const entries = ${JSON.stringify(entries)}; const load = name => import(entries[name]); ${observation}`);
    const actual = successful(execute(process.execPath, [generatedDriver], root), 'same-runtime emitted');
    assert.equal(actual, baseline);
    assert.equal(actual, 'init\ntrue true 0 true\n1 1\ntrue true true\n');

    const diagnostics = [
      ['nonliteral', { 'entry.mjs': "const name = './dep.mjs'; import(name);" }, /nonliteral|literal/iu],
      ['external', { 'entry.mjs': "import 'node:fs';" }, /external.*explicit/iu],
      ['require-async', {
        'entry.cjs': "require('./async.mjs');",
        'async.mjs': "await Promise.resolve(); export const value = 1;",
      }, /require.*async|async.*require/isu],
      ['typescript', { 'entry.ts': 'export const value: number = 1;', 'package.json': '{"type":"module"}' }, /JavaScript only|TS\/JSX/iu],
    ];
    for (const [name, sources, expected] of diagnostics) {
      const project = path.join(directory, name);
      writeSources(project, sources);
      const result = execute(driver, [project, path.join(project, '.bundler-output'), Object.keys(sources)[0]], project, 30000);
      assert.equal(result.status, 1, `${name}: diagnostic build unexpectedly passed`);
      assert.match(result.stderr, expected, `${name}: wrong diagnostic: ${result.stderr}`);
      assert.match(result.stderr, /entry\.(?:mjs|cjs|ts)/u, `${name}: missing source path`);
      assert.match(result.stderr, /bytes \d+\.\.\d+/u, `${name}: missing source range`);
      assert(!existsSync(path.join(project, '.bundler-output/.bundler-current.json')), `${name}: failed build published`);
    }
    return { identity: 1, diagnostics: diagnostics.length };
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const results = runDifferential(process.argv[2] ?? defaultRoot);
    for (const result of results) console.log(`${result.status === 'passed' ? 'PASS differential' : 'UNSUPPORTED'} ${result.id}${result.backend ? ` backend=${result.backend}` : ''}${result.reason ? `: ${result.reason}` : ''}`);
    const extra = runBundlerCases();
    console.log(`Differential: ${results.filter((r) => r.status === 'passed').length} passed, ${results.filter((r) => r.status === 'unsupported').length} unsupported, ${results.reduce((n,r) => n + r.executions, 0)} emitted executions; ${extra.identity} same-runtime identity case; ${extra.diagnostics} diagnostic cases.`);
  } catch (error) {
    console.error(error.stack ?? error);
    process.exitCode = 1;
  }
}
