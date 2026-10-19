console.log('before rejection');
await Promise.reject(new RangeError('authored rejection'));
