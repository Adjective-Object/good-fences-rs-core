import assert from 'node:assert/strict';
import {
  cpSync, existsSync, lstatSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const defaultRoot = fileURLToPath(new URL('../fixtures/bundler/', import.meta.url));
const requiredFields = ['id', 'upstream', 'moduleType', 'entryPoints', 'expectations', 'goals'];

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function object(value, fields, label) {
  check(value !== null && typeof value === 'object' && !Array.isArray(value),
    `${label} must be an object`);
  check(Object.keys(value).sort().join('\0') === [...fields].sort().join('\0'),
    `${label} must contain exactly: ${fields.join(', ')}`);
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
    throw new Error(`Invalid JSON in ${filename}: ${error.message}`);
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
  object(manifest, requiredFields, 'case.json');
  relativePath(manifest.id, 'id');
  const relative = path.relative(root, directory).split(path.sep).join('/');
  check(manifest.id === relative && relative.includes('/'), `id must match upstream/case directory: ${relative}`);
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
  check(['module', 'commonjs'].includes(manifest.moduleType), 'moduleType must be module or commonjs');
  strings(manifest.entryPoints, 'entryPoints');
  check(new Set(manifest.entryPoints).size === manifest.entryPoints.length, 'entryPoints must be unique');
  object(manifest.expectations, ['stdout'], 'expectations');
  check(Array.isArray(manifest.expectations.stdout) && manifest.expectations.stdout.every(
    (line) => typeof line === 'string' && !/[\r\n]/u.test(line)),
  'expectations.stdout must be an array of individual output lines');
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
        throw new Error(`${directory}: ${error.message}`);
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

export function runFixtures(root = defaultRoot, { timeoutMs = 5000, tempRoot = tmpdir() } = {}) {
  check(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'timeoutMs must be a positive integer');
  // Validate the entire corpus before executing any third-party source.
  const cases = discoverCases(root);
  return cases.map(({ manifest, files }) => {
    const directory = mkdtempSync(path.join(tempRoot, 'bundler-fixture-'));
    try {
      cpSync(files, directory, { recursive: true });
      if (!existsSync(path.join(directory, 'package.json'))) {
        writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ type: manifest.moduleType }));
      }
      const stdout = [];
      const env = { ...process.env };
      delete env.NODE_OPTIONS;
      delete env.NODE_PATH;
      delete env.NODE_V8_COVERAGE;
      for (const entry of manifest.entryPoints) {
        const result = spawnSync(process.execPath, ['--', path.join(directory, entry)], {
          cwd: directory, env, encoding: 'utf8', timeout: timeoutMs,
          killSignal: 'SIGKILL', maxBuffer: 1024 * 1024,
        });
        const label = `${manifest.id} (${entry})`;
        check(!result.error, `${label}: ${result.error?.message}`);
        check(result.status === 0 && result.signal === null,
          `${label}: abnormal termination (exit ${result.status}, signal ${result.signal})${result.stderr ? `\n${result.stderr}` : ''}`);
        check(result.stderr === '', `${label}: unexpected stderr:\n${result.stderr}`);
        stdout.push(...outputLines(result.stdout));
      }
      assert.deepEqual(stdout, manifest.expectations.stdout, `${manifest.id}: stdout mismatch`);
      return { id: manifest.id, stdout };
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    check(process.argv.length <= 3, 'Usage: node scripts/bundler-fixtures.mjs [fixture-root]');
    const results = runFixtures(process.argv[2]);
    results.forEach(({ id }) => console.log(`PASS baseline ${id}`));
    console.log(`Validated ${results.length} runtime semantic contracts; no bundler conformance tested.`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
