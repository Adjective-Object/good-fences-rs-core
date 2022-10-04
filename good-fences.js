#! /usr/bin/env node
const { goodFences } = require('./index');
const { program } = require('commander');


program
    .option('-p, --project <string> ', 'tsconfig.json file path, defaults `./tsconfig.json`')
    .option('-o, --output <string>', 'path to write found violations')
    .option('--baseUrl <string>', "Overrides `compilerOptions.baseUrl` property read from '--project' argument")
    .arguments('<path> [morePaths...]', 'Dirs to look for fence and source files')
program.parse(process.argv);

const options = program.opts();
const args = program.args;

const result = goodFences(args, options.project, options.baseUrl, options.output)
