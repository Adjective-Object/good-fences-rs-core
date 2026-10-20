try {
  await import('./entry.mjs');
} catch (error) {
  console.log(error.message);
}
try {
  console.log(saved.x);
} catch (error) {
  console.log(error.name);
}
