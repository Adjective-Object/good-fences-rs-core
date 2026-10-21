import './z.mjs';
import './a.mjs';
export {value, bump} from './a.mjs';
const secret = (trace.push('child'), {});
export function read() {
  return secret;
}
