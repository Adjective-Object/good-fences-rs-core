function commonjsNamespace(value) {
  const namespace = intrinsics.create(null);
  intrinsics.defineProperty(namespace, 'default', { enumerable: true, value });
  if (value != null) {
    const target = intrinsics.object(value);
    const keys = intrinsics.getOwnPropertyNames(target);
    for (let index = 0; index < keys.length; index++) {
      const key = keys[index];
      if (key !== 'default') {
        intrinsics.defineProperty(namespace, key, { enumerable: true, get: () => target[key] });
      }
    }
  }
  intrinsics.defineProperty(namespace, intrinsics.toStringTag, { value: 'Module' });
  return intrinsics.freeze(namespace);
}
