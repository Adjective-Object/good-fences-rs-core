import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { pathToFileURL, fileURLToPath } from 'node:url';

export async function observe({ entries, entry, directory }) {
  const links = [];
  globalThis.document = {
    createElement(name) { assert.equal(name, 'link'); return {}; },
    head: { appendChild(link) { links.push(link); } },
  };
  try {
    const module = await import(pathToFileURL(entries[entry]));
    assert.equal(links.length, 0);
    const mapping = await module.load();
    assert.equal(links.length, 2);
    assert.equal(await module.load(), mapping);
    assert.equal(links.length, 2);
    const tokens = mapping.button.split(' ');
    assert.equal(tokens.length, 2);
    for (const [index, link] of links.entries()) {
      assert.equal(link.rel, 'stylesheet');
      assert(link.href.startsWith(pathToFileURL(`${directory}/`).href));
      assert(existsSync(fileURLToPath(link.href)));
      const css = readFileSync(fileURLToPath(link.href), 'utf8');
      assert(css.includes(`.${tokens[index === 0 ? 1 : 0]}`));
    }
    const consumer = readFileSync(fileURLToPath(links[1].href), 'utf8');
    assert.match(consumer, /@import .* print;/u);
    console.log('lazy dependency-first once-only mounts');
  } finally {
    delete globalThis.document;
  }
}
