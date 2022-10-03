import test from 'ava'

import {goodFences} from '../index.js'

test('sum from native', (t) => {
  t.assert(
    goodFences(["tests/good_fences_integration/src"], "tests/good_fences_integration/tsconfig.json", undefined, undefined).length === 4
  )
})
