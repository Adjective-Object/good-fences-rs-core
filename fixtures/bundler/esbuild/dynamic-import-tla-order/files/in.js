function foo() {
  globalThis.tlaTrace.push(2)
  return import('./a.js')
}

globalThis.tlaTrace = []
globalThis.tlaTrace.push(1)
const it = (await foo()).default
globalThis.tlaTrace.push(6)
console.log(it, globalThis.tlaTrace.join(','))
