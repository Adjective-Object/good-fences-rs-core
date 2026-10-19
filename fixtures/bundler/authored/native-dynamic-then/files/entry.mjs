import { later, namespace } from './consumer.mjs';
console.log('before', namespace.value, namespace.default, Object.keys(namespace).join(','));
const pending = later();
console.log('pending');
Promise.resolve().then(() => console.log('microtask'));
pending.then(async value => {
  console.log('after', value === namespace.bump, namespace.value, namespace.default);
  console.log('again', (await later()) === value, namespace.value, namespace.default);
});
