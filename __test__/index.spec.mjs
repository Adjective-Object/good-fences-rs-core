import test from 'ava'

import {goodFences} from '../index.js'

test('run tests/good_fences_integration through napi', (t) => {
  t.assert(
    goodFences({
      paths: ["tests/good_fences_integration/src"],
      project: "tests/good_fences_integration/tsconfig.json",
    }).length === 4
  )
})
