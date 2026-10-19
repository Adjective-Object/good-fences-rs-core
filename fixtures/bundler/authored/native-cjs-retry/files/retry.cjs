module.exports = () => {
  try { require('./fail.cjs'); }
  catch (error) { return error.message; }
};
