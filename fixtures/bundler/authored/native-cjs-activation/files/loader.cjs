module.exports = () => {
  const value = require('./deferred.cjs');
  return value === require('./shared.cjs').object ? value : null;
};
