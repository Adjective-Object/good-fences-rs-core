import test from 'ava'

import {goodFences} from '../index.js'

test('sum from native', (t) => {
  goodFences(["packages", "shared"], "../client-web/tsconfig.paths.json", ".", "../client-web", undefined);
})
