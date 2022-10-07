import test from 'ava'

import {goodFences} from '../index.js'

test('sum from native', (t) => {
  t.assert(
    goodFences({
      paths: ["tests/good_fences_integration/src"],
      project: "tests/good_fences_integration/tsconfig.json",
    }).length === 4
  )
})
