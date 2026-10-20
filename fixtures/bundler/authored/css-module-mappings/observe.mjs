import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export async function observe({ entries, entry, directory, stylesheets }) {
  const mapping = await import(pathToFileURL(entries[entry]));
  assert.deepEqual(Object.keys(mapping), ['buttonLabel']);
  assert.match(mapping.buttonLabel, /^_[A-Za-z0-9_-]{10}$/u);
  const css = readFileSync(path.join(directory, Object.values(stylesheets)[0]), 'utf8');
  assert(css.includes(`.${mapping.buttonLabel}, #${mapping.buttonLabel}`));
  assert(css.includes(`.theme .${mapping.buttonLabel}`));
  assert(css.includes('color: green'));
  assert(!css.includes('.unused-rule'));
  assert(!css.includes(':global'));
  console.log('mappings and stylesheet retained');
}
