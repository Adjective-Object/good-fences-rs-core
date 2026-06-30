#! /usr/bin/env node
// @ts-check

/**
 * `./index` is generated via `napi build` or `yarn build` along with `.node`
 * It contains js/ts friendly definitions of rust code annotated with `#[napi]`
 */
const { goodFences, GoodFencesResultType } = require('./index');
const { program } = require('commander');


program
    .option('-p, --project <string> ', 'tsconfig.json file path, defaults to `./tsconfig.json`', './tsconfig.paths.json')
    .option('-o, --output <string>', 'path to write found violations')
    .option('--baseUrl <string>', "Overrides `compilerOptions.baseUrl` property read from '--project' argument")
    .option('-q, --quiet', 'Suppress unresolved file warnings', false)
    .option('--ignoreExternalFences', 'Ignore external fences (e.g. those in `node_modules`)', false)
    .option('--ignoredDirs [pathRegexs...]', 'Directories matching given regular expressions are excluded from fence evaluation (e.g. `--ignoreDirs lib` will not evaluate source files in all dirs named `lib`', [])
    .arguments('<path> [morePaths...]')
program.parse(process.argv);

const options = program.opts();
const args = program.args;

const opts = {
    paths: args ?? ['packages', 'shared'],
    project: options.project,
    errOutputPath: options.output,
    ignoreExternalFences: options.ignoreExternalFences ? 1 : 0,
    ignoredDirs: options.ignoredDirs,
};
if (options.baseUrl) {
    opts.baseUrl = options.baseUrl;
}

const result = goodFences(opts);

const violations = [];
const unresolved = [];
result.forEach(r => {
    if (r.resultType === GoodFencesResultType.Violation) {
        violations.push(r.detailedMessage);
    } else {
        unresolved.push(r.detailedMessage);
    }
});

if (violations.length) {
    console.error(`${violations.length} violation(s) found:\n`);
    violations.forEach(v => console.error(v));
}

if (!options.quiet && unresolved.length) {
    console.log(`\n${unresolved.length} unresolved import(s):\n`);
    unresolved.forEach(u => console.log(u));
}

if (violations.length) {
    process.exit(1);
}