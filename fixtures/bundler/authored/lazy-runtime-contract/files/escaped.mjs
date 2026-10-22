import {trace} from './trace.mjs';
trace.push('escaped');
export function value() { return 20; }
export function extra() { return 'ESCAPED_BROAD_NAMESPACE'; }
