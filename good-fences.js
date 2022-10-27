#! /usr/bin/env node

/**
 * `./index` is generated via `napi build` or `yarn build` along with `.node`
 * It contains js/ts friendly definitions of rust code annotated with `#[napi]`
 */
 const { goodFences } = require('./index');
 const { program } = require('commander');
 
 
 program
    .option('-p, --project <string> ', 'tsconfig.json file path, defaults `./tsconfig.json`', 'tsconfig.json')
    .option('-o, --output <string>', 'path to write found violations')
    .option('--baseUrl <string>', "Overrides `compilerOptions.baseUrl` property read from '--project' argument")
    .option('--ignoreExternalFences', 'Ignore external fences (e.g. those in `node_modules`)', false)
    .option('--ignoredDirs [pathRegexs...]', 'Files under directories matching given regular expressions are excluded from fence evaluation and will not generate an import map but they are going to be included in the list sources and a fence configuration is still attached to their path (e.g. `--ignoreDirs lib` will not evaluate source files in all dirs named `lib`)', [])
    .arguments('<path> [morePaths...]', 'Dirs to look for fence and source files')
program.parse(process.argv);
 
const options = program.opts();
const args = program.args;

const result = goodFences({
    paths: args ?? [],
    project: options.project,
    baseUrl: options.baseUrl,
    errOutputPath: options.output,
    ignoreExternalFences: options.ignoreExternalFences ? 1 : 0,
    ignoredDirs: options.ignoredDirs
});

result.forEach(r => {
    console.error(r.detailedMessage);
});
 