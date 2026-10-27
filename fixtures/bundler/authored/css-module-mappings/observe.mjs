import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export async function observe({ entries, entry, directory, stylesheets }) {
  const mapping = await import(pathToFileURL(entries[entry]));
  assert.deepEqual(Object.keys(mapping), ['button-label']);
  assert.match(mapping['button-label'], /^_[A-Za-z0-9_-]{10}$/u);
  const styles = Object.entries(stylesheets);
  const module = styles.find(([source]) => source.endsWith('style.module.css'))[1];
  const global = styles.find(([source]) => source.endsWith('theme.global.css'))[1];
  assert(readFileSync(path.join(directory, global), 'utf8').includes('.external-shell'));
  const css = readFileSync(path.join(directory, module), 'utf8');
  assert(css.includes(`.${mapping['button-label']}, #${mapping['button-label']}`));
  assert(css.includes(`.theme .${mapping['button-label']}`));
  assert(css.includes('color: green'));
  assert(!css.includes('.unused-rule'));
  assert(!css.includes(':global'));
  console.log('mappings and stylesheet retained');
}
