use std::path::Path;

use crate::owned::{CommonJsGlobal, DependencyKind};
use crate::{parse_module, ModuleBody, ModuleFormat, ParsedModule};

fn parse(source: &str, path: &str) -> eyre::Result<ParsedModule> {
    parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        ModuleFormat::Esm,
    )
}

#[test]
fn opaque_body_preserves_directives_reassignment_and_nested_sites() {
    let source = "#!/usr/bin/env node\n'use strict';\n\
        const café = require('./same');\n\
        module.exports = café; exports = {replacement: true};\n\
        function load() { return require('./same'); }\n\
        const lazy = () => import('./async');\n\
        module['exports'].load = load;\n\
        if (café) return; require('./after-return');";
    let module = parse(source, "test.cjs").unwrap();
    let span = module.segments[0].span;
    assert_eq!(module.body, ModuleBody::CommonJs { span });
    assert_eq!(module.segments.len(), 1);
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        &source[source.find('\n').unwrap()..]
    );
    assert_eq!(module.facts.directives[0].value, "use strict");
    assert_eq!(module.facts.dependencies.len(), 4);
    assert_eq!(module.facts.dependencies[0].enclosing_function, None);
    for dependency in &module.facts.dependencies[1..3] {
        let function = dependency.enclosing_function.unwrap();
        assert!(function.start <= dependency.span.start && function.end >= dependency.span.end);
    }
    assert_eq!(module.facts.dependencies[3].enclosing_function, None);
    assert!(module
        .facts
        .references
        .iter()
        .any(|r| r.commonjs == Some(CommonJsGlobal::Module)));
    assert!(module
        .facts
        .references
        .iter()
        .any(|r| r.commonjs == Some(CommonJsGlobal::Exports) && r.write));
    assert_eq!(module.segments[0].module_deps.requires.len(), 2);
    assert_eq!(module.segments[0].module_deps.dynamic_imports.len(), 1);
    assert!(module.facts.exports.is_empty());
    assert_eq!(module.facts, parse(source, "test.cjs").unwrap().facts);
    assert!(module.facts.ecco.is_none());
    let facts = crate::owned::ModuleFacts {
        ecco_declarations_hash: Default::default(),
        ..module.facts
    };
    insta::assert_debug_snapshot!(facts);
}

#[test]
fn global_wrapper_names_are_distinct_from_lexical_shadows() {
    let module = parse(
        "function shadow(require, module, exports) {\n\
            require(path); module.exports = exports;\n\
         }\n\
         function lexical() { require('./outer'); const require = loader; }\n\
         function hoisted() { require('./outer'); var require; }\n\
         function closure() { return require('./real'); }\n\
         module.exports = () => exports;",
        "shadow.cjs",
    )
    .unwrap();
    assert_eq!(module.facts.dependencies.len(), 1);
    assert_eq!(module.facts.dependencies[0].specifier, "./real");
    for name in ["require", "module", "exports"] {
        assert!(module
            .facts
            .references
            .iter()
            .any(|r| r.name == name && r.binding.is_some() && r.commonjs.is_none()));
        assert!(module
            .facts
            .references
            .iter()
            .any(|r| r.name == name && r.binding.is_none() && r.commonjs.is_some()));
    }
    let module = parse(
        "type require = string; interface module {x: number} type exports = number;\n\
         module.exports = require('./runtime'); exports.x = 1;",
        "types.cts",
    )
    .unwrap();
    assert_eq!(module.facts.dependencies.len(), 1);
    assert_eq!(module.facts.dependencies[0].specifier, "./runtime");
    assert!(module.facts.references.iter().all(|r| r.commonjs.is_some()));
}

#[test]
fn commonjs_empty_bodies_and_top_level_return_are_deliberate() {
    for source in [
        "",
        "'use strict';",
        "return 1;",
        "module.exports = 1; return;",
    ] {
        let module = parse(source, "test.cjs").unwrap();
        assert_eq!(module.segments.len(), 1);
        assert_eq!(module.segments[0].span.start, 0);
        assert_eq!(module.segments[0].span.end as usize, source.len());
    }
    let module = parse(
        "const x: number = 1; module.exports = x; return;",
        "test.cts",
    )
    .unwrap();
    assert!(matches!(module.body, ModuleBody::CommonJs { .. }));
    assert!(parse("return;", "test.mjs").is_err());
    assert!(parse("'use strict'; with (value) {}", "strict.cjs").is_err());
    for path in ["file.js", "file.ts", "file.jsx", "file.tsx"] {
        let module = parse_module(
            &logger::StdioLogger::new(),
            "module.exports = 1;",
            Path::new(path),
            ModuleFormat::CommonJs,
        )
        .unwrap();
        assert!(matches!(module.body, ModuleBody::CommonJs { .. }));
        assert_eq!(module.segments.len(), 1);
    }
    let module = parse_module(
        &logger::StdioLogger::new(),
        "export const x = 1;",
        Path::new("file.mjs"),
        ModuleFormat::CommonJs,
    )
    .unwrap();
    assert_eq!(module.body, ModuleBody::Esm);
}

#[test]
fn require_shapes_and_mutation_have_span_diagnostics() {
    for (source, expected) in [
        ("require(path)", "nonliteral require"),
        ("require()", "require call"),
        ("require('./x', sideEffect())", "require call"),
        ("require(...paths)", "nonliteral require"),
        ("require?.('./x')", "require call"),
        ("require = loader; require('./x');", "require reassignment"),
        ("const load = require; load('./x');", "require reassignment"),
        ("require.resolve('./x');", "require reassignment"),
        ("module.require('./x');", "module access"),
        ("const m = module; m.require('./x');", "module access"),
        ("module[name]('./x');", "module access"),
        ("var require; require('./x');", "wrapper redeclaration"),
        ("var module; module.exports = 1;", "wrapper redeclaration"),
        ("function exports() {}", "wrapper redeclaration"),
        ("let __filename = 'local';", "wrapper redeclaration"),
        ("const __dirname = 'local';", "wrapper redeclaration"),
        ("class __filename {}", "wrapper redeclaration"),
        ("arguments[1]('./x');", "wrapper arguments"),
        (
            "const load = () => arguments[1]('./x');",
            "wrapper arguments",
        ),
    ] {
        let error = format!("{:#}", parse(source, "bad.cjs").unwrap_err());
        assert!(
            error.contains("bad.cjs") && error.contains("bytes ") && error.contains(expected),
            "{}: {}",
            source,
            error
        );
    }
    let module = parse(
        "(require)(('./x')); import(('./y')); module = {}; module.exports = 1;",
        "valid.cjs",
    )
    .unwrap();
    assert_eq!(module.facts.dependencies[0].specifier, "./x");
    assert_eq!(module.facts.dependencies[1].specifier, "./y");
    assert!(module
        .facts
        .references
        .iter()
        .any(|r| r.commonjs == Some(CommonJsGlobal::Module) && r.write));
}

#[test]
fn commonjs_path_parameters_allow_var_function_and_nested_shadowing() {
    for source in [
        "var __filename; var __dirname; console.log(__filename, __dirname);",
        "function __filename() {} function __dirname() {}",
        "function shadow(__filename, __dirname) { return [__filename, __dirname]; }",
        "{ const __filename = 'local'; let __dirname = 'local'; }",
        "if (false) { var __filename; var __dirname; }",
    ] {
        parse(source, "paths.cjs").unwrap();
    }
}

#[test]
fn mixed_modules_are_errors_not_partial_records() {
    for (path, source) in [
        ("test.mjs", "export const x = require('./x');"),
        ("test.mjs", "export const x = 1; module.exports = x;"),
        ("test.mjs", "exports.x = 1;"),
        ("test.cjs", "import x from './x'; module.exports = x;"),
        ("test.cjs", "export const x = 1;"),
        ("test.cjs", "export default 1;"),
    ] {
        let error = format!("{:#}", parse(source, path).unwrap_err());
        // OXC can reject a mixed declaration before the extraction-policy gate.
        assert!(error.contains(path), "{}", error);
        assert!(
            error.contains("mixed") || error.contains("diagnostics"),
            "{}",
            error
        );
    }
    assert!(parse(
        "const require = loader; require(path); const module = {}; module.exports = 1;",
        "shadow.mjs"
    )
    .is_ok());
}

#[test]
fn esm_top_level_await_facts_exclude_function_bodies_and_cjs_remains_sync() {
    let source = "await (await 1); for await (const x of stream) { await x; } async function f() { await 2; for await (const y of stream) {} } const g = async () => await 3;";
    let parsed = parse(source, "async.mjs").unwrap();
    let sites: Vec<_> = parsed
        .facts
        .top_level_await
        .iter()
        .map(|s| &source[s.start as usize..s.end as usize])
        .collect();
    assert_eq!(
        sites,
        [
            "await (await 1)",
            "await 1",
            "for await (const x of stream) { await x; }",
            "await x"
        ]
    );
    assert!(parse("await 1;", "async.cjs").is_err());
    assert!(parse("for await (const x of stream) {}", "async.cjs").is_err());
    assert!(parse("async function f() { await 1; }", "sync.cjs")
        .unwrap()
        .facts
        .top_level_await
        .is_empty());
}

#[test]
fn unsupported_transforms_and_execution_features_are_diagnostics() {
    for (path, source, expected) in [
        ("test.jsx", "export const x = <div/>;", "JSX transform"),
        ("test.tsx", "export default <>text</>;", "JSX transform"),
        ("test.ts", "enum Color {Red}", "enum transform"),
        ("test.ts", "const enum Color {Red}", "enum transform"),
        (
            "test.ts",
            "namespace N { export const x = 1; }",
            "namespace transform",
        ),
        (
            "test.ts",
            "class C { constructor(public x: number) {} }",
            "parameter property",
        ),
        ("test.ts", "@decorate class C {}", "decorator transform"),
        (
            "test.ts",
            "class C { accessor x = 1; }",
            "auto-accessor transform",
        ),
        ("test.js", "using x = resource();", "resource-management"),
        (
            "test.mjs",
            "await using x = resource();",
            "resource-management",
        ),
        ("test.mjs", "console.log(import.meta.url);", "import.meta"),
        (
            "test.cjs",
            "function f() { with (object) {} }",
            "with statement",
        ),
        ("test.cjs", "eval('require(path)');", "unshadowed eval"),
        ("test.cjs", "(0, eval)('code');", "unshadowed eval"),
        (
            "test.mjs",
            "import x from './x' with {type: 'json'};",
            "import attributes",
        ),
        ("test.mjs", "import source x from './x';", "import phase"),
        (
            "test.mjs",
            "export * from './x' with {type: 'json'};",
            "import attributes",
        ),
        (
            "test.mjs",
            "import('./x', {with: {type: 'json'}});",
            "dynamic import options",
        ),
        (
            "test.ts",
            "type T = import('./x', {with: {type: 'json'}}).T;",
            "import type options",
        ),
    ] {
        let error = format!("{:#}", parse(source, path).unwrap_err());
        assert!(
            error.contains(path) && error.contains("bytes ") && error.contains(expected),
            "{}: {}",
            source,
            error
        );
    }
    assert!(parse(
        "async function f() { await import('./x'); } const g = async () => await import('./y');",
        "valid.mjs"
    )
    .is_ok());
    assert!(parse(
        "function f(eval) { return eval('ordinary call'); }",
        "valid.cjs"
    )
    .is_ok());
    assert!(parse("function f() { return arguments[0]; }", "valid.cjs").is_ok());
}

#[test]
fn owned_utf8_spans_cover_declarations_references_and_dependency_sites() {
    let source = "const café = require('./dépendance'); module.exports = café;";
    let module = parse(source, "unicode.cjs").unwrap();
    let binding = &module.facts.bindings[0];
    assert_eq!(
        &source[binding.declarations[0].start as usize..binding.declarations[0].end as usize],
        "café"
    );
    for reference in &module.facts.references {
        assert_eq!(
            &source[reference.span.start as usize..reference.span.end as usize],
            reference.name
        );
    }
    let dependency = &module.facts.dependencies[0];
    assert_eq!(
        &source[dependency.specifier_span.start as usize..dependency.specifier_span.end as usize],
        "'./dépendance'"
    );
    assert_eq!(
        &source[dependency.span.start as usize..dependency.span.end as usize],
        "require('./dépendance')"
    );
}

#[test]
fn parse_only_target_facts_are_not_resolution_or_transformation_claims() {
    let module = parse(
        "type T = import('./types').T; const value: T = input;\n\
         require('./native.node'); require('./esm.mjs'); import('./style.css'); import('node:fs');\n\
         module.exports = value;",
        "targets.cts",
    ).unwrap();
    assert_eq!(module.facts.dependencies.len(), 5);
    assert_eq!(
        module
            .facts
            .dependencies
            .iter()
            .map(|d| d.specifier.as_str())
            .collect::<Vec<_>>(),
        [
            "./types",
            "./native.node",
            "./esm.mjs",
            "./style.css",
            "node:fs"
        ]
    );
    assert_eq!(
        module.facts.dependencies[0].kind,
        DependencyKind::ImportType
    );
    assert!(module.facts.dependencies[0].type_only);
    assert_eq!(module.segments[0].module_deps.requires.len(), 2);
    assert_eq!(module.segments[0].module_deps.dynamic_imports.len(), 2);
}
