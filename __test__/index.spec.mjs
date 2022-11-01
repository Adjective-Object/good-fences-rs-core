import test from 'ava'

import {goodFences, GoodFencesResultType} from '../index.js'

test('run tests/good_fences_integration through napi', (t) => {
  const result = goodFences({
    paths: ["tests/good_fences_integration/src"],
    project: "tests/good_fences_integration/tsconfig.json",
  });
  t.is(result.filter(r => r.resultType !== GoodFencesResultType.Violation).length, 0)
  t.is(result.filter(r => r.resultType === GoodFencesResultType.Violation).length, 6)
})

test('run tests/good_fences_integration through napi ignoring componentA', (t) => {
  const result = goodFences({
    paths: ["tests/good_fences_integration/src"],
    project: "tests/good_fences_integration/tsconfig.json",
    ignoredDirs: ['componentA'],
  });
  t.is(result.filter(r => r.resultType !== GoodFencesResultType.Violation).length, 1);
  t.is(result.filter(r => r.resultType === GoodFencesResultType.Violation).length, 2);
})

test('run tests/good_fences_integration through napi ignoring componentA and complexComponentA', (t) => {
  const result = goodFences({
    paths: ["tests/good_fences_integration/src"],
    project: "tests/good_fences_integration/tsconfig.json",
    ignoredDirs: ['componentA', 'complexComponentA'],
  });
  t.is(result.filter(r => r.resultType !== GoodFencesResultType.Violation).length, 1);
  t.is(result.filter(r => r.resultType === GoodFencesResultType.Violation).length, 1);
})

