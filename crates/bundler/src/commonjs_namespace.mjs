function commonjsNamespace(value) {
  const namespace = Object.create(null);
  Object.defineProperty(namespace, 'default', { enumerable: true, value });
  if (value != null) {
    const target = Object(value);
    for (const key of Object.getOwnPropertyNames(target)) {
      if (key !== 'default') {
        Object.defineProperty(namespace, key, { enumerable: true, get: () => target[key] });
      }
    }
  }
  Object.defineProperty(namespace, Symbol.toStringTag, { value: 'Module' });
  return Object.freeze(namespace);
}
