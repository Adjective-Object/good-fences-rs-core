import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export async function observe({ entries, entry, directory, stylesheets, resources }) {
  const links = [];
  globalThis.document = {
    createElement() { return {}; },
    head: { appendChild(link) { links.push(link); } },
  };
  try {
    const module = await import(pathToFileURL(entries[entry]));
    const ownedCss = Object.values(stylesheets)[0];
    const ownedImage = resources.find(name => name.endsWith('.svg'));
    const base = 'https://cdn.example.test/static/';
    assert.equal(module.stylesheet, base + ownedCss);
    assert.equal(module.image, base + ownedImage);
    assert.deepEqual(links, [{ rel: 'stylesheet', href: module.stylesheet }]);
    const css = readFileSync(path.join(directory, ownedCss), 'utf8');
    assert(css.includes(`${module.image}?theme=dark#symbol`));
    for (const unchanged of ['https://other.example.test/remote.svg', 'data:image/svg+xml,icon', '#filter']) {
      assert(css.includes(unchanged));
    }
    assert.equal(readFileSync(path.join(directory, ownedImage), 'utf8'),
      '<svg xmlns="http://www.w3.org/2000/svg"><symbol id="symbol"/></svg>\n');
    console.log('public CSS and asset URLs preserve owned bytes');
  } finally {
    delete globalThis.document;
  }
}
