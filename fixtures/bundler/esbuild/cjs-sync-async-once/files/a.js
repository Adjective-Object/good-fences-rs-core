import * as ns1 from './b.cjs'
export default async function () {
  const ns2 = await import('./b.cjs')
  return [ns1.foo, -ns2.default.foo]
}
