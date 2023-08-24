let unused = require('./index').findUnusedItems;

unused(['packages', 'shared'], './tsconfig.paths.json', ["**/test/**", "**/owa-fluent-icons-svg/**", "**/__generated__/**"], [".*Props$", ".*Tests$"]);