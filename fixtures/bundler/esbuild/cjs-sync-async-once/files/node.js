const {default: fn} = await import('./a.js')
const [a, b] = await fn()
console.log(a, b)
