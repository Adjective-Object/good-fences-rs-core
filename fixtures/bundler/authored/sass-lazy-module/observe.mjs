import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';

export async function observe({entries, entry, directory}) {
  const links = [];
  globalThis.document = {
    createElement(name) { assert.equal(name, 'link'); return {}; },
    head: { appendChild(link) { links.push(link); } },
  };
  try {
    const module = await import(pathToFileURL(entries[entry]));
    assert.equal(module.host, 'browser');
    assert.equal(links.length, 0);
    const mapping = await module.load();
    assert.equal(links.length, 1);
    assert.equal(await module.load(), mapping);
    assert.equal(links.length, 1);
    assert.match(mapping.button, /^_[A-Za-z0-9_-]{10}$/u);
    const link = links[0];
    assert.equal(link.rel, 'stylesheet');
    assert(link.href.startsWith(pathToFileURL(`${directory}/`).href));
    const css = readFileSync(fileURLToPath(link.href), 'utf8');
    assert(css.includes(`.${mapping.button}`));
    assert(css.includes(`.${mapping.button}:hover`));
    assert.match(css, /padding: 6px/u);
    assert.match(css, /color: red/u);
    const [, asset] = /url\("([^"]+)"\)/u.exec(css);
    assert.match(readFileSync(fileURLToPath(new URL(asset, link.href)), 'utf8'), /<svg/u);
    assert.doesNotMatch(css, /@use|@mixin|\$color/u);
    console.log('native Sass deferred mappings and resources');
  } finally {
    delete globalThis.document;
  }
}
