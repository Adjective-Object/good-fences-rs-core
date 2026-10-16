import * as ns1 from './b.js'
export default async function () {
  const ns2 = await import('./b.js')
  return [ns1.foo, -ns2.foo]
}
