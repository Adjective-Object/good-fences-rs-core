import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';

const exec = promisify(execFile);
const root = fileURLToPath(new URL('../', import.meta.url));
const corpus = path.join(root, 'fixtures', 'bundler-apps', 'todomvc');
const driver = path.join(root, 'target', 'debug', 'examples', `build${process.platform === 'win32' ? '.exe' : ''}`);
const browser = process.env.AGENT_BROWSER_BIN || 'agent-browser';
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css' };
const errorCapture = `window.__bundlerBrowserErrors = [];
window.addEventListener('error', event => {
  window.__bundlerBrowserErrors.push(event.message || 'Resource failed: ' + (event.target.src || event.target.href));
}, true);
window.addEventListener('unhandledrejection', event => {
  window.__bundlerBrowserErrors.push(String(event.reason));
});`;

const apps = [
  { id: 'javascript-es6', input: '.new-todo', items: '.todo-list > li', title: 'label',
    toggle: '.toggle', edit: '.edit', remove: '.destroy', all: '.toggle-all-label',
    clear: '.clear-completed', count: '.todo-count', persistent: false, newestFirst: true },
  { id: 'web-components', input: '.new-todo-input', items: 'todo-item', title: '.todo-item-text',
    toggle: '.toggle-todo-input', edit: '.edit-todo-input', remove: '.remove-todo-button',
    all: '.toggle-all-label', clear: '.clear-completed-button', count: '.todo-status', persistent: false },
];

export function adaptVanilla(source) {
  source = source.replace(/(from "\.\/[^".]+)(")/gu, '$1.js$2');
  source = source.replace(/^import "[^"]+\.css";\r?\n/gmu, '');
  const hmr = /\/\* HOT MODULE SPECIFIC \*\/[\s\S]*?(?=window\.addEventListener\("load")/u;
  if (source.includes('module.hot')) {
    assert.match(source, hmr, 'Expected pinned webpack HMR block');
    source = source.replace(hmr, '');
  }
  return source;
}

export async function verifyCorpus(directory = corpus) {
  const hashes = JSON.parse(await readFile(path.join(directory, 'sha256.json'), 'utf8'));
  assert.ok(Object.keys(hashes).length > 0, 'Empty source inventory');
  const actual = [];
  async function walk(current, prefix = '') {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const relative = prefix + entry.name;
      if (entry.isDirectory()) await walk(path.join(current, entry.name), `${relative}/`);
      else {
        assert.ok(entry.isFile(), `Nonregular fixture: ${relative}`);
        actual.push(relative);
      }
    }
  }
  await walk(path.join(directory, 'upstream'), 'upstream/');
  await walk(path.join(directory, 'licenses'), 'licenses/');
  assert.deepEqual(actual.sort(), Object.keys(hashes).sort(), 'Source inventory differs');
  for (const [filename, hash] of Object.entries(hashes)) {
    assert.ok(!filename.includes('\\') && filename.split('/').every((part) => part && part !== '..' && part !== '.'),
      `Invalid fixture path: ${filename}`);
    assert.equal(createHash('sha256').update(await readFile(path.join(directory, filename))).digest('hex'),
      hash, `Changed pinned input: ${filename}`);
  }
}

export async function prepare(app, directory) {
  const original = path.join(corpus, 'upstream', 'examples', app.id);
  await mkdir(directory, { recursive: true });
  await cp(path.join(original, 'src'), path.join(directory, 'src'), { recursive: true });
  let html;
  let styles;
  if (app.id === 'javascript-es6') {
    for (const filename of await readdir(path.join(directory, 'src'))) {
      if (!filename.endsWith('.js')) continue;
      const target = path.join(directory, 'src', filename);
      await writeFile(target, adaptVanilla(await readFile(target, 'utf8')));
    }
    await cp(path.join(original, 'dist', 'app.css'), path.join(directory, 'app.css'));
    html = await readFile(path.join(original, 'src', 'index.html'), 'utf8');
    html = html.replace('</head>', '<link rel="stylesheet" href="app.css" /></head>');
    await writeFile(path.join(directory, 'entry.js'), 'import "./src/app.js";\n');
    styles = ['app.css'];
  } else {
    await cp(path.join(original, 'styles'), path.join(directory, 'styles'), { recursive: true });
    await cp(path.join(original, 'dist', 'styles', 'base.css'), path.join(directory, 'styles', 'base.css'));
    html = await readFile(path.join(original, 'index.html'), 'utf8');
    html = html.replace('node_modules/todomvc-common/base.css', 'styles/base.css');
    const components = ['todo-topbar', 'todo-list', 'todo-bottombar', 'todo-app'];
    await writeFile(path.join(directory, 'entry.js'), components.map(
      (name) => `import "./src/components/${name}/${name}.component.js";\n`).join(''));
    styles = ['styles/global.css', 'styles/header.css', 'styles/footer.css', 'styles/base.css'];
  }
  html = html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/gu, '');
  html = html.replace('</head>', '<link rel="icon" href="data:," /></head>');
  html = html.replace('</head>', `<script>${errorCapture}</script></head>`);
  html = html.replace('</body>', '<script type="module" src="entry.js"></script></body>');
  await writeFile(path.join(directory, 'index.html'), html);
  await writeFile(path.join(directory, 'package.json'), '{"type":"module"}\n');
  return { html, styles };
}

export async function serve(mounts) {
  const requests = [];
  const server = createServer(async (request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    try {
      const [, mount, ...parts] = decodeURIComponent(pathname).split('/');
      assert.ok(Object.hasOwn(mounts, mount), 'Unknown mount');
      assert.ok(parts.every((part) => part !== '..' && !part.includes('\\') && !part.includes('\0')), 'Invalid path');
      const filename = path.resolve(mounts[mount], parts.join(path.sep) || 'index.html');
      assert.ok(filename.startsWith(`${path.resolve(mounts[mount])}${path.sep}`), 'Path escapes mount');
      const bytes = await readFile(filename);
      requests.push({ path: pathname, status: 200 });
      response.writeHead(200, { 'Content-Type': `${mime[path.extname(filename)] || 'application/octet-stream'}; charset=utf-8`,
        'Cache-Control': 'no-store' });
      response.end(bytes);
    } catch (error) {
      requests.push({ path: pathname, status: 404, error: error.message });
      response.writeHead(404);
      response.end('Not found');
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return { server, requests, url: `http://127.0.0.1:${server.address().port}` };
}

async function agent(session, ...args) {
  let stdout;
  try {
    ({ stdout } = await exec(browser, ['--session', session, '--json', ...args], {
      timeout: 45_000, maxBuffer: 4 * 1024 * 1024,
      env: { ...process.env, AGENT_BROWSER_DEFAULT_TIMEOUT: '10000' },
    }));
  } catch (error) {
    if (!error.stdout?.trim().startsWith('{')) throw error;
    const result = JSON.parse(error.stdout);
    throw new Error(`${args[0]}: ${result.error || error.message}`, { cause: error });
  }
  const result = JSON.parse(stdout);
  assert.equal(result.success, true, `${args[0]}: ${result.error}`);
  return result.data;
}

function observation(app) {
  // Query real open shadow roots for Web Components, without altering the app.
  return `(() => {
    function all(selector, scope = document) {
      const found = [...scope.querySelectorAll(selector)];
      for (const element of scope.querySelectorAll('*')) {
        if (element.shadowRoot) found.push(...all(selector, element.shadowRoot));
      }
      return found;
    }
    const visible = element => element.checkVisibility();
    return {
      items: all(${JSON.stringify(app.items)}).filter(visible).map(element => ({
        title: all(${JSON.stringify(app.title)}, element.shadowRoot || element)[0].textContent,
        completed: all(${JSON.stringify(app.toggle)}, element.shadowRoot || element)[0].checked
      })),
      count: all(${JSON.stringify(app.count)}).find(visible)?.textContent || '',
      hash: location.hash
    };
  })()`;
}

async function exercise(app, base, output, mode) {
  const session = `bundler-${app.id}-${mode}-${process.pid}`;
  const trace = [];
  const diagnostics = {};
  let failure;
  const shadow = app.id === 'web-components';
  const item = (index, total) => `${app.items}:nth-child(${app.newestFirst ? total + 1 - index : index})`;
  async function reference(role, name, index = 0) {
    const { refs, snapshot } = await agent(session, 'snapshot');
    const matches = [...snapshot.matchAll(/ref=(e\d+)/gu)].map((match) => match[1])
      .filter((ref) => refs[ref]?.role === role && refs[ref].name === name);
    assert.ok(matches[index], `Missing ${role} ${name} #${index}`);
    return `@${matches[index]}`;
  }
  async function edit(title) {
    if (shadow) {
      await agent(session, 'focus', await reference('checkbox', 'Toggle Todo', 1));
      await agent(session, 'press', 'Tab');
      await agent(session, 'press', 'Space');
      await agent(session, 'find', 'role', 'textbox', 'fill', title, '--name', 'Edit todo', '--exact');
    } else {
      await agent(session, 'dblclick', `${item(2, 3)} ${app.title}`);
      await agent(session, 'fill', `${item(2, 3)} ${app.edit}`, title);
    }
  }
  async function checkpoint(name, titles, completed, active) {
    if (app.newestFirst) {
      titles = [...titles].reverse();
      completed = completed && [...completed].reverse();
    }
    const expression = observation(app);
    await agent(session, 'wait', '--fn',
      `(() => {
        if (window.__bundlerBrowserErrors.length) throw new Error(window.__bundlerBrowserErrors.join('\\n'));
        return JSON.stringify((${expression}).items.map(item => item.title)) === ${JSON.stringify(JSON.stringify(titles))};
      })()`);
    const state = (await agent(session, 'eval', expression)).result;
    assert.deepEqual(state.items.map((item) => item.title), titles, name);
    if (completed) assert.deepEqual(state.items.map((item) => item.completed), completed, name);
    if (active !== undefined) assert.equal(parseInt(state.count, 10), active, `${name}: active count`);
    trace.push({ name, ...state });
  }
  async function add(title) {
    if (shadow) await agent(session, 'find', 'role', 'textbox', 'fill', title, '--name', 'Enter a new todo.', '--exact');
    else await agent(session, 'fill', app.input, title);
    await agent(session, 'press', 'Enter');
    // The ES6 implementation commits new todos on the native change event.
    await agent(session, 'press', 'Tab');
  }
  try {
    await agent(session, 'open', base);
    if (shadow) await agent(session, 'wait', '--fn', 'document.querySelector("todo-app")?.isReady === true');
    else await agent(session, 'wait', app.input);
    diagnostics.userAgent = (await agent(session, 'eval', 'navigator.userAgent')).result;
    await checkpoint('empty', []);
    await add('buy milk');
    await add('write tests');
    await add('<b>literal text</b>');
    await checkpoint('add and literal text', ['buy milk', 'write tests', '<b>literal text</b>'], [false, false, false], 3);
    await agent(session, 'click', shadow ? await reference('checkbox', 'Toggle Todo') : `${item(1, 3)} ${app.toggle}`);
    await checkpoint('complete one', ['buy milk', 'write tests', '<b>literal text</b>'], [true, false, false], 2);
    await agent(session, 'find', 'role', 'link', 'click', '--name', 'Active', '--exact');
    await checkpoint('active route', ['write tests', '<b>literal text</b>'], [false, false], 2);
    await agent(session, 'find', 'role', 'link', 'click', '--name', 'Completed', '--exact');
    await checkpoint('completed route', ['buy milk'], [true], 2);
    await agent(session, 'find', 'role', 'link', 'click', '--name', 'All', '--exact');
    await checkpoint('all route', ['buy milk', 'write tests', '<b>literal text</b>']);
    await edit('write browser tests');
    await agent(session, 'press', 'Enter');
    await checkpoint('edit commit', ['buy milk', 'write browser tests', '<b>literal text</b>']);
    await edit('discard this edit');
    await agent(session, 'press', 'Escape');
    await checkpoint('edit cancel', ['buy milk', 'write browser tests', '<b>literal text</b>']);
    await agent(session, 'screenshot', path.join(output, `${mode}.png`), '--full');
    await agent(session, 'find', 'role', 'button', 'click', '--name', 'Clear completed', '--exact');
    await checkpoint('clear completed', ['write browser tests', '<b>literal text</b>'], [false, false], 2);
    if (shadow) {
      await agent(session, 'hover', await reference('checkbox', 'Toggle Todo', 1));
      await agent(session, 'click', await reference('button', '×'));
    } else {
      await agent(session, 'hover', item(2, 2));
      await agent(session, 'click', `${item(2, 2)} ${app.remove}`);
    }
    await checkpoint('delete one', ['write browser tests'], [false], 1);
    await agent(session, 'click', shadow
      ? await reference('checkbox', '❯ Mark all todos as complete.') : app.all);
    await checkpoint('toggle all complete', ['write browser tests'], [true], 0);
    await agent(session, 'click', shadow
      ? await reference('checkbox', '❯ Mark all todos as complete.') : app.all);
    await checkpoint('toggle all active', ['write browser tests'], [false], 1);
    await agent(session, 'reload');
    if (shadow) await agent(session, 'wait', '--fn', 'document.querySelector("todo-app")?.isReady === true');
    else await agent(session, 'wait', app.input);
    await checkpoint(app.persistent ? 'reload persists' : 'reload resets (upstream has no persistence)',
      app.persistent ? ['write browser tests'] : []);
  } catch (error) {
    failure = error.message;
  } finally {
    // Preserve diagnostics even when an interaction fails; do not lose the original failure.
    for (const [name, command] of Object.entries({
      snapshot: ['snapshot'], console: ['console'], errors: ['errors'], network: ['network', 'requests'],
      browserErrors: ['eval', 'window.__bundlerBrowserErrors'],
    })) {
      try { diagnostics[name] = await agent(session, ...command); }
      catch (error) { diagnostics[`${name}Failure`] = error.message; }
    }
    if (failure) {
      try { await agent(session, 'screenshot', path.join(output, `${mode}-failure.png`), '--full'); }
      catch (error) { diagnostics.screenshotFailure = error.message; }
    }
    try { await agent(session, 'close'); }
    catch (error) { diagnostics.closeFailure = error.message; }
  }
  const consoleErrors = diagnostics.console?.messages.filter((message) => message.type === 'error') || [];
  const pageErrors = [...(diagnostics.errors?.errors || []), ...(diagnostics.browserErrors?.result || [])];
  const diagnosticFailures = Object.keys(diagnostics).filter((name) => name.endsWith('Failure'));
  if (diagnosticFailures.length) failure = [failure, ...diagnosticFailures.map((name) => `${name}: ${diagnostics[name]}`)]
    .filter(Boolean).join('\n');
  return { trace, diagnostics, failure, consoleErrors, pageErrors };
}

export async function run({ output = path.join(root, 'target', 'bundler-browser') } = {}) {
  await verifyCorpus();
  await mkdir(output, { recursive: true });
  const tempRoot = path.join(root, 'target', 'p1-test-tmp');
  await mkdir(tempRoot, { recursive: true });
  const workspace = await mkdtemp(path.join(tempRoot, 'todomvc-'));
  const results = [];
  try {
    for (const app of apps) {
      const result = { app: app.id, status: 'FAIL', stage: 'input' };
      const artifacts = path.join(output, app.id);
      await mkdir(artifacts, { recursive: true });
      let server;
      try {
        const source = path.join(workspace, app.id);
        const { html, styles } = await prepare(app, source);
        const serving = await serve({ source });
        server = serving.server;
        const sourceResult = await exercise(app, `${serving.url}/source/`, artifacts, 'source');
        result.source = sourceResult;
        assert.ok(!sourceResult.failure, `Source behavior: ${sourceResult.failure}`);
        assert.equal(sourceResult.consoleErrors.length + sourceResult.pageErrors.length, 0, 'Source browser errors');
        result.stage = 'build';
        const { stdout, stderr } = await exec(driver, [source, path.join(workspace, `${app.id}-published`),
          '--css-auto-mount', 'entry.js', ...styles],
          { cwd: root, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
        assert.equal(stderr, '', 'Build stderr');
        const build = JSON.parse(stdout);
        result.build = build;
        result.stage = 'backend';
        assert.equal(build.backend, 'native', 'Real apps require the native backend');
        result.stage = 'output';
        // Keep published output for inspection, but never overlay files from an older run.
        const generation = await mkdtemp(path.join(artifacts, 'bundle-'));
        await cp(build.directory, generation, { recursive: true });
        const entryUrl = path.relative(build.directory, build.entries['entry.js']).split(path.sep).join('/');
        let bundledHtml = html.replace('src="entry.js"', `src="${entryUrl}"`);
        for (const stylesheet of styles) {
          const resource = Object.entries(build.stylesheets).find(([filename]) =>
            path.relative(path.toNamespacedPath(source), filename) === path.normalize(stylesheet))?.[1];
          assert.ok(resource, `Missing emitted stylesheet: ${stylesheet}`);
          bundledHtml = bundledHtml.replace(`href="${stylesheet}"`, `href="${resource}"`);
        }
        await writeFile(path.join(generation, 'index.html'), bundledHtml);
        result.bundle = generation;
        result.stage = 'output';
        await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
        server = undefined;
        const bundledServing = await serve({ bundle: generation });
        server = bundledServing.server;
        result.output = await exercise(app, `${bundledServing.url}/bundle/`, artifacts, 'output');
        assert.ok(!result.output.failure, `Bundled behavior: ${result.output.failure}`);
        assert.equal(result.output.consoleErrors.length + result.output.pageErrors.length, 0, 'Bundled browser errors');
        assert.deepEqual(result.output.trace, sourceResult.trace, 'Source/bundle behavior differs');
        assert.ok([...serving.requests, ...bundledServing.requests].every((request) => request.status === 200),
          'Failed HTTP requests');
        result.status = 'PASS';
        delete result.stage;
      } catch (error) {
        result.error = error.message;
      } finally {
        if (server) await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
      }
      results.push(result);
      await writeFile(path.join(artifacts, 'result.json'), `${JSON.stringify(result, null, 2)}\n`);
      console.log(`${result.status} ${app.id}${result.stage ? ` [${result.stage}]: ${result.error}` : ''}`);
    }
  } finally {
    await rm(workspace, { recursive: true, force: true });
  }
  await writeFile(path.join(output, 'results.json'), `${JSON.stringify(results, null, 2)}\n`);
  return results;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const results = await run();
  if (results.some((result) => result.status !== 'PASS')) process.exitCode = 1;
}
