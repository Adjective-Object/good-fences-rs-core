import {trace} from './trace.mjs';
trace.push('untrusted');
export function value() { return 30; }
export function extra() { return 'UNTRUSTED_BROAD_NAMESPACE'; }
