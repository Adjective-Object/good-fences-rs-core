/** @typedef {import("beachball/lib/types/BeachballOptions").RepoOptions} RepoOptions */

/** @type {RepoOptions} */
module.exports = {
  branch: "main",
  access: "public",
  registry: `https://registry.npmjs.org/:_authToken=${process.env.NPM_TOKEN}`
};
