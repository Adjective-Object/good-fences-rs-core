#! /usr/bin/env node
// @ts-check

/**
 * `./index` is generated via `napi build` or `yarn build` along with `.node`
 * It contains js/ts friendly definitions of rust code annotated with `#[napi]`
 */
const { goodFences, GoodFencesResultType } = require('./index');
const { program } = require('commander');


program
    .option('-o, --output <string>', 'path to write found violations')
    .option('--ignoreExternalFences', 'Ignore external fences (e.g. those in `node_modules`)', false)
    .option('--ignoredDirs [pathRegexs...]', 'Directories matching given regular expressions are excluded from fence evaluation (e.g. `--ignoreDirs lib` will not evaluate source files in all dirs named `lib`', [])
    .arguments('<path> [morePaths...]')
program.parse(process.argv);

const options = program.opts();
const args = program.args;

const result = goodFences({
    paths: args ?? ['packages', 'shared'],
    errOutputPath: options.output,
    ignoreExternalFences: options.ignoreExternalFences ? 1 : 0,
    ignoredDirs: options.ignoredDirs,
});

result.forEach(r => {
    if (r.resultType !== GoodFencesResultType.Violation) {
        console.log(r.detailedMessage);
    }

    if (r.resultType === GoodFencesResultType.Violation) {
        console.error(r.detailedMessage);
    }
});

if (result.find(r => r.resultType === GoodFencesResultType.Violation)) {
    process.exit(1);
}