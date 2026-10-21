(() => {
  const descriptor = Object.getOwnPropertyDescriptor;
  const hasOwn = Object.hasOwn;
  return error => {
    const message = descriptor(error, "message");
    return message && hasOwn(message, "value") && typeof message.value === "string"
      ? message.value
      : undefined;
  };
})()
