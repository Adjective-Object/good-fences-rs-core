import test from 'ava'

import { goodFences } from '../index.js'

test('sum from native', (t) => {
  goodFences(["packages", "shared"], "../tsconfig.paths.json", ".", "../client-web", undefined);
})
