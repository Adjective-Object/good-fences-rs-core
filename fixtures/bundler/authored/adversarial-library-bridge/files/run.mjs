export async function run(reverse) {
  globalThis.trace = [];
  const a = async () => (await import('./root-a.mjs')).read();
  const b = async () => (await import('./root-b.mjs')).read();
  const c = async () => (await import('./root-c.mjs')).read();
  const d = async () => (await import('./root-d.mjs')).read();
  console.log('cold', trace.length);
  const loaders = reverse ? [d, c, b, a] : [a, b, c, d];
  const labels = reverse ? ['d', 'c', 'b', 'a'] : ['a', 'b', 'c', 'd'];
  let first;
  for (let i = 0; i < loaders.length; i++) {
    const result = await loaders[i]();
    if (result.some(item => item.identity === null || typeof item.identity !== 'object')) throw new Error('identity erased');
    if (i === 0) first = result[0];
    console.log(labels[i], trace.join(','), result.map(item => item.count).join(':'));
  }
  const again = reverse ? await d() : await a();
  first.bump(9);
  console.log('retained', first.identity === again[0].identity, first.read(), trace.join(','));
}
