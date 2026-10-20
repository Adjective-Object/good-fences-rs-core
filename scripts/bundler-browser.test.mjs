import assert from 'node:assert/strict';
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { adaptVanilla, prepare, serve, verifyCorpus } from './bundler-browser.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));

async function temporary(t) {
  const parent = path.join(root, 'target', 'p1-test-tmp');
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(path.join(parent, 'browser-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

test('pinned apps adapt only the browser packaging, preserving application logic', async (t) => {
  const directory = await temporary(t);
  for (const id of ['javascript-es6', 'web-components']) {
    const source = path.join(directory, id);
    const { html, styles } = await prepare({ id }, source);
    assert.match(html, /type="module" src="entry\.js"/u);
    assert.doesNotMatch(html, /src="\.\/base\.js"|node_modules/u);
    for (const stylesheet of styles) assert.ok((await readFile(path.join(source, stylesheet))).length);
    if (id === 'javascript-es6') {
      const app = await readFile(path.join(source, 'src', 'app.js'), 'utf8');
      assert.match(app, /from "\.\/view\.js"/u);
      assert.match(app, /window\.addEventListener\("load", onLoad\)/u);
      assert.doesNotMatch(app, /module\.hot|import "[^"]+\.css"/u);
    } else {
      const filename = path.join('src', 'components', 'todo-app', 'todo-app.component.js');
      assert.equal(await readFile(path.join(source, filename), 'utf8'),
        await readFile(path.join(root, 'fixtures', 'bundler-apps', 'todomvc', 'upstream', 'examples', id, filename), 'utf8'));
    }
  }
  assert.throws(() => adaptVanilla('if (module.hot) somethingElse();'), /Expected pinned webpack HMR block/u);
});

test('source inventory rejects altered or unlisted inputs before browser execution', async (t) => {
  await verifyCorpus();
  const directory = await temporary(t);
  await cp(path.join(root, 'fixtures', 'bundler-apps', 'todomvc'), directory, { recursive: true });
  const filename = path.join(directory, 'upstream', 'examples', 'javascript-es6', 'src', 'app.js');
  await writeFile(filename, 'changed');
  await assert.rejects(verifyCorpus(directory), /Changed pinned input/u);
  await writeFile(path.join(directory, 'upstream', 'unexpected.js'), '');
  await assert.rejects(verifyCorpus(directory), /Source inventory differs/u);
});

test('static server uses native module MIME types and reports missing and escaping requests', async (t) => {
  const directory = await temporary(t);
  await writeFile(path.join(directory, 'index.html'), '<h1>app</h1>');
  await writeFile(path.join(directory, 'entry.mjs'), 'export const value = 1;');
  await writeFile(path.join(directory, 'style.css'), 'body { color: red; }');
  const { server, url, requests } = await serve({ app: directory });
  t.after(() => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())));
  for (const [filename, type] of [['', 'text/html'], ['entry.mjs', 'text/javascript'], ['style.css', 'text/css']]) {
    const response = await fetch(`${url}/app/${filename}`);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('Content-Type'), `${type}; charset=utf-8`);
    await response.text();
  }
  for (const filename of ['missing.js', '..%5Csecret', '%00.js']) {
    const response = await fetch(`${url}/app/${filename}`);
    assert.equal(response.status, 404);
    await response.text();
  }
  assert.equal(requests.filter((request) => request.status === 404).length, 3);
});
