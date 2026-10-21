import {read as current, bump} from './state.mjs';
import {writeImport} from './writer.mjs';
export function read() {
  const error = writeImport();
  const before = current();
  bump();
  return [error, before];
}
