globalThis.trace = [];
console.log('cold', trace.length);
let first;
try { await import('./root-a.mjs'); } catch (error) { first = error; }
console.log('x', first.name, first.message, trace.join(','));
console.log('good', (await import('./root-d.mjs')).read(), trace.join(','));
let again;
let sibling;
try { await import('./root-a.mjs'); } catch (error) { again = error; }
try { await import('./root-b.mjs'); } catch (error) { sibling = error; }
console.log('cached', first === again, first === sibling, trace.join(','));
let one = false;
let two = false;
try { await import('./root-c.mjs'); } catch (error) { one = error === undefined; }
try { await import('./root-c.mjs'); } catch (error) { two = error === undefined; }
console.log('y', one, two, trace.join(','));
console.log('still-good', (await import('./root-d.mjs')).read(), trace.join(','));
