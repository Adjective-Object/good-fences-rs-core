import test from 'ava'

import {goodFences} from '../index.js'

test('run tests/good_fences_integration through napi', (t) => {
  t.is(
    goodFences({
      paths: ["tests/good_fences_integration/src"],
      project: "tests/good_fences_integration/tsconfig.json",
    }).length, 6
  )
})

test('run tests/good_fences_integration through napi ignoring componentA', (t) => {
  t.is(
    goodFences({
      paths: ["tests/good_fences_integration/src"],
      project: "tests/good_fences_integration/tsconfig.json",
      ignoredDirs: ['componentA']
    }).length, 2
  )
})

test('run tests/good_fences_integration through napi ignoring componentA and complexComponentA', (t) => {
  t.is(
    goodFences({
      paths: ["tests/good_fences_integration/src"],
      project: "tests/good_fences_integration/tsconfig.json",
      ignoredDirs: ['componentA', 'complexComponentA'],
    }).length, 1
  )
})

