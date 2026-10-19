import assert from 'node:assert/strict';
import {
  cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const defaultRoot = fileURLToPath(new URL('../fixtures/bundler/', import.meta.url));
const requiredFields = ['id', 'moduleType', 'entryPoints', 'expectations', 'goals'];
const defaultTempRoot = fileURLToPath(new URL('../target/p1-test-tmp/', import.meta.url));
const defaultDriver = fileURLToPath(new URL(`../target/debug/examples/build${process.platform === 'win32' ? '.exe' : ''}`, import.meta.url));

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value, fields, label, optional = []) {
  check(value !== null && typeof value === 'object' && !Array.isArray(value),
    `${label} must be an object`);
  check(fields.every((field) => Object.hasOwn(value, field))
    && Object.keys(value).every((field) => fields.includes(field) || optional.includes(field)),
  `${label} must contain exactly required fields: ${fields.join(', ')}; optional: ${optional.join(', ')}`);
}

function text(value, label) {
  check(typeof value === 'string' && value.trim().length > 0, `${label} must be nonempty text`);
}

function strings(value, label, { empty = false } = {}) {
  check(Array.isArray(value) && (empty || value.length > 0), `${label} must be an array${empty ? '' : ' with at least one item'}`);
  value.forEach((item) => text(item, label));
}

function relativePath(value, label) {
  text(value, label);
  check(!/[\\:\0]/u.test(value) && value.split('/').every((part) => part && part !== '.' && part !== '..'),
    `${label} must be a contained, slash-separated relative path: ${value}`);
}

function regularFile(filename) {
  check(lstatSync(filename).isFile(), `Expected a regular file (no symlinks): ${filename}`);
}

function json(filename) {
  regularFile(filename);
  try {
    return JSON.parse(readFileSync(filename, 'utf8'));
  } catch (error) {
    throw new Error(`Invalid JSON in ${filename}: ${error.message}`, { cause: error });
  }
}

function validateFiles(directory) {
  check(lstatSync(directory).isDirectory(), `Expected a directory (no symlinks): ${directory}`);
  for (const name of readdirSync(directory).sort()) {
    const filename = path.join(directory, name);
    relativePath(name, 'source filename');
    check(name !== 'node_modules', `External dependency trees are unsupported: ${filename}`);
    const stat = lstatSync(filename);
    if (stat.isDirectory()) validateFiles(filename);
    else regularFile(filename);
  }
}

function validateCase(root, directory) {
  const manifest = json(path.join(directory, 'case.json'));
  object(manifest, requiredFields, 'case.json', ['upstream']);
  relativePath(manifest.id, 'id');
  const relative = path.relative(root, directory).split(path.sep).join('/');
  check(manifest.id === relative && relative.includes('/'), `id must match group/case directory: ${relative}`);
  if (Object.hasOwn(manifest, 'upstream')) {
    const upstream = manifest.upstream;
    object(upstream, ['repository', 'commit', 'paths', 'test', 'license', 'adaptations'], 'upstream');
    check(typeof upstream.repository === 'string' && /^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(upstream.repository),
      'upstream.repository must be a canonical HTTPS GitHub repository URL');
    check(typeof upstream.commit === 'string' && /^[a-f0-9]{40}$/iu.test(upstream.commit),
      'upstream.commit must be a full 40-character commit SHA');
    strings(upstream.paths, 'upstream.paths');
    upstream.paths.forEach((item) => relativePath(item, 'upstream.paths'));
    text(upstream.test, 'upstream.test');
    text(upstream.license, 'upstream.license');
    strings(upstream.adaptations, 'upstream.adaptations', { empty: true });
    const license = path.join(root, relative.split('/')[0], 'LICENSE');
    regularFile(license);
    text(readFileSync(license, 'utf8'), 'upstream LICENSE');
  }
  check(['module', 'commonjs'].includes(manifest.moduleType), 'moduleType must be module or commonjs');
  strings(manifest.entryPoints, 'entryPoints');
  check(new Set(manifest.entryPoints).size === manifest.entryPoints.length, 'entryPoints must be unique');
  object(manifest.expectations, ['stdout'], 'expectations', ['errors', 'backend']);
  check(Array.isArray(manifest.expectations.stdout) && manifest.expectations.stdout.every(
    (line) => typeof line === 'string' && !/[\r\n]/u.test(line)),
  'expectations.stdout must be an array of individual output lines');
  if (Object.hasOwn(manifest.expectations, 'backend')) {
    check(manifest.expectations.backend === 'native', 'expectations.backend must be native');
  }
  if (Object.hasOwn(manifest.expectations, 'errors')) {
    const errors = manifest.expectations.errors;
    object(errors, [], 'expectations.errors', manifest.entryPoints);
    check(Object.keys(errors).length > 0, 'expectations.errors must not be empty');
    for (const [entry, error] of Object.entries(errors)) {
      object(error, ['name'], `expectations.errors.${entry}`, ['code']);
      text(error.name, 'error.name');
      if (Object.hasOwn(error, 'code')) text(error.code, 'error.code');
    }
  }
  strings(manifest.goals, 'goals');
  const files = path.join(directory, 'files');
  validateFiles(files);
  for (const entry of manifest.entryPoints) {
    relativePath(entry, 'entryPoints');
    regularFile(path.join(files, entry));
  }
  const packageFile = path.join(files, 'package.json');
  if (existsSync(packageFile)) {
    const metadata = json(packageFile);
    check(metadata !== null && typeof metadata === 'object' && !Array.isArray(metadata),
      'files/package.json must be an object');
    check(metadata.type === undefined || ['module', 'commonjs'].includes(metadata.type),
      'files/package.json type must be module or commonjs when present');
  }
  return { manifest, files };
}

export function discoverCases(root = defaultRoot) {
  root = path.resolve(root);
  const cases = [];
  function visit(directory) {
    check(lstatSync(directory).isDirectory(), `Expected a directory (no symlinks): ${directory}`);
    if (existsSync(path.join(directory, 'case.json'))) {
      try {
        cases.push(validateCase(root, directory));
      } catch (error) {
        throw new Error(`${directory}: ${error.message}`, { cause: error });
      }
      return;
    }
    check(!existsSync(path.join(directory, 'files')), `Missing case.json beside ${directory}/files`);
    for (const name of readdirSync(directory).sort()) {
      const filename = path.join(directory, name);
      const stat = lstatSync(filename);
      check(!stat.isSymbolicLink(), `Symlinks are unsupported: ${filename}`);
      if (stat.isDirectory()) visit(filename);
      else regularFile(filename);
    }
  }
  visit(root);
  check(cases.length > 0, `No fixture cases found in ${root}`);
  return cases.sort((a, b) => a.manifest.id < b.manifest.id ? -1 : a.manifest.id > b.manifest.id ? 1 : 0);
}

function outputLines(stdout) {
  if (stdout === '') return [];
  const lines = stdout.split(/\r?\n/u);
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

export function managed(tempRoot, callback) {
  mkdirSync(tempRoot, { recursive: true });
  const directory = mkdtempSync(path.join(tempRoot, 'bundler-fixture-'));
  try { return callback(directory); }
  finally { rmSync(directory, { recursive: true, force: true }); }
}

export function execute(command, args, cwd, timeoutMs = 5000, observation = false) {
  check(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'timeoutMs must be a positive integer');
  const env = { ...process.env };
  for (const name of ['NODE_OPTIONS', 'NODE_PATH', 'FORCE_COLOR']) delete env[name];
  // Node forwards the parent's coverage path even when omitted from env.
  env.NODE_V8_COVERAGE = '';
  const result = spawnSync(command, args, {
    cwd, env, encoding: 'utf8', timeout: timeoutMs,
    killSignal: 'SIGKILL', maxBuffer: 1024 * 1024,
    ...(observation ? { stdio: ['ignore', 'pipe', 'pipe', 'pipe'] } : {}),
  });
  if (result.error) {
    result.error.childPid = result.pid;
    throw result.error;
  }
  return result;
}

export function successful(result, label) {
  check(result.status === 0 && result.signal === null,
    `${label}: abnormal termination (exit ${result.status}, signal ${result.signal})${result.stderr ? `\n${result.stderr}` : ''}`);
  check(result.stderr === '', `${label}: unexpected stderr:\n${result.stderr}`);
  return result.stdout;
}

export function copyCase(files, directory, manifest) {
  cpSync(files, directory, { recursive: true });
  if (!existsSync(path.join(directory, 'package.json'))) {
    writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ type: manifest.moduleType }));
  }
  if (manifest.expectations.errors) {
    // Parse failures are not expected runtime throws/rejections.
    function checkSyntax(folder) {
      for (const name of readdirSync(folder).sort()) {
        const filename = path.join(folder, name);
        if (lstatSync(filename).isDirectory()) checkSyntax(filename);
        else if (/\.(?:js|mjs|cjs)$/u.test(name)) {
          successful(execute(process.execPath, ['--check', filename], directory),
            `${manifest.id}: source syntax validation (${name})`);
        }
      }
    }
    checkSyntax(directory);
  }
}

// A separate pipe carries caught import completion, never fixture stdout or stacks.
const observer = `
import { writeSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const entry = process.argv[1];
let outcome = null;
try { await import(pathToFileURL(entry).href); }
catch (error) {
  outcome = {name: error instanceof Error ? error.name : typeof error};
  if (typeof error?.code === 'string') outcome.code = error.code;
}
writeSync(3, JSON.stringify(outcome));
`;

export function observeEntries(manifest, entries, cwd, timeoutMs = 5000, mode = 'baseline') {
  const stdout = [];
  const outcomes = [];
  for (const entry of manifest.entryPoints) {
    const errors = manifest.expectations.errors;
    const expected = errors && Object.hasOwn(errors, entry) ? errors[entry] : undefined;
    const label = `${manifest.id} ${mode} (${entry})`;
    const result = execute(process.execPath,
      ['--input-type=module', '-e', observer, '--', entries[entry]], cwd, timeoutMs, true);
    const lines = outputLines(successful(result, label));
    stdout.push(...lines);
    check(result.output[3], `${label}: observer did not complete${expected ? '; expected runtime failure did not occur' : ''}`);
    const actual = JSON.parse(result.output[3]);
    if (expected) {
      check(actual !== null, `${label}: expected runtime failure did not occur`);
      assert.equal(actual.name, expected.name, `${label}: runtime error name mismatch`);
      if (Object.hasOwn(expected, 'code')) {
        assert.equal(actual.code, expected.code, `${label}: runtime error code mismatch`);
      }
    } else {
      assert.equal(actual, null, `${label}: unexpected runtime failure: ${JSON.stringify(actual)}`);
    }
    outcomes.push({ entry, stdout: lines, error: actual });
  }
  assert.deepEqual(stdout, manifest.expectations.stdout, `${manifest.id}: ${mode} stdout mismatch`);
  return { stdout, outcomes };
}

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

export function runFixtures(root = defaultRoot, {
  driver = defaultDriver, timeoutMs = 5000, tempRoot = defaultTempRoot, unsupported = new Map(),
} = {}) {
  check(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'timeoutMs must be a positive integer');
  // Validate the entire corpus before executing any third-party source.
  let cases;
  try { cases = discoverCases(root); }
  catch (cause) { throw new Error(`INPUT metadata: ${cause.message}`, { cause }); }
  assert(unsupported instanceof Map, 'unsupported must map case IDs to explicit reasons');
  for (const [id, reason] of unsupported) {
    assert(cases.some(({ manifest }) => manifest.id === id), `unknown unsupported case: ${id}`);
    assert.equal(typeof reason, 'string');
    assert(reason.trim(), 'unsupported case needs a reason');
  }
  return cases.map(({ manifest, files }) => managed(tempRoot, (directory) => {
    let stage = 'input';
    let backend;
    try {
      const native = path.join(directory, 'native');
      const source = path.join(directory, 'source');
      copyCase(files, native, manifest);
      const baseline = observeEntries(manifest,
        Object.fromEntries(manifest.entryPoints.map((entry) => [entry, path.join(native, entry)])),
        native, timeoutMs, 'source');
      if (unsupported.has(manifest.id)) {
        return { id: manifest.id, status: 'unsupported', reason: unsupported.get(manifest.id), executions: 0 };
      }
      stage = 'build';
      copyCase(files, source, manifest);
      const build = buildProject(driver, source, manifest.entryPoints);
      backend = build.backend;
      stage = 'backend';
      if (manifest.expectations.backend === 'native') {
        assert.equal(backend, 'native', `${manifest.id}: native backend required; legacy fallback selected`);
      }
      stage = 'output';
      const bundled = observeEntries(manifest, build.entries, source, timeoutMs, 'bundled');
      assert.deepEqual(bundled, baseline, `${manifest.id}: emitted/source runtime observation mismatch`);
      return { id: manifest.id, status: 'passed', backend, executions: manifest.entryPoints.length, stdout: baseline.stdout };
    } catch (error) {
      return { id: manifest.id, status: 'failed', stage, ...(backend ? { backend } : {}), executions: 0, error };
    }
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

function diagnostic(error) {
  return `${error.stack ?? error}${error.cause ? `\nCaused by: ${diagnostic(error.cause)}` : ''}`;
}

function report(result) {
  const color = process.env.NO_COLOR === undefined
    && (process.env.FORCE_COLOR === undefined ? process.stdout.isTTY : process.env.FORCE_COLOR !== '0');
  const paint = (text, code) => color ? `\x1b[${code}m${text}\x1b[0m` : text;
  const status = result.status === 'passed' ? paint('PASS', 32)
    : result.status === 'failed' ? `${paint('FAIL', 31)} ${result.stage.toUpperCase()}` : 'UNSUPPORTED';
  const backend = result.backend === 'legacy' ? paint('backend=legacy', 31) : `backend=${result.backend}`;
  return `${status} ${result.id}${result.backend ? ` ${backend}` : ''}${result.reason ? `: ${result.reason}` : ''}`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    check(process.argv.length <= 3, 'Usage: node scripts/bundler-fixtures.mjs [fixture-root]');
    const results = runFixtures(process.argv[2]);
    for (const result of results) {
      console.log(report(result));
      if (result.error) console.error(diagnostic(result.error));
    }
    const count = (status, backend) => results.filter((r) => r.status === status && (!backend || r.backend === backend)).length;
    console.log(`Fixtures: ${count('passed')} passed (${count('passed', 'native')} native, ${count('passed', 'legacy')} legacy), ${count('failed')} failed, ${count('unsupported')} unsupported, ${results.reduce((n, r) => n + r.executions, 0)} emitted executions.`);
    if (count('failed')) process.exitCode = 1;
    const extra = runBundlerCases();
    console.log(`Bundler checks: ${extra.identity} same-runtime identity case; ${extra.diagnostics} diagnostic cases.`);
  } catch (error) {
    console.error(`Runner error: ${diagnostic(error)}`);
    process.exitCode = 1;
  }
}
