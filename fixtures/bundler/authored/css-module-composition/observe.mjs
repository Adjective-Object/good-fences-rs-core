import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export async function observe({ entries, entry, directory, resources }) {
  const mapping = await import(pathToFileURL(entries[entry]));
  assert.deepEqual(mapping.default, { button: mapping.button, localBase: mapping.localBase });
  const tokens = mapping.button.split(' ');
  assert.match(tokens[0], /^_[A-Za-z0-9_-]{10}$/u);
  assert.deepEqual(tokens.slice(1), [mapping.localBase, mapping.baseName, 'external-token']);
  assert.equal(new Set(tokens).size, tokens.length);
  for (const resource of resources.filter(name => name.endsWith('.css'))) {
    assert(!readFileSync(path.join(directory, resource), 'utf8').includes('composes:'));
  }
  console.log('composition order and deduplication');
}
