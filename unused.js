let unused = require('./index').findUnusedItems;

unused(['packages', 'shared'], './tsconfig.paths.json', ['**/__generated__/**'], [".*Props$"]);