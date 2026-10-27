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
fn json_import_attributes_preserve_static_dependency_evidence() {
    for source in [
        "import data from './data.json' with { type: 'json' }; export default data;",
        "import {default as data} from './data' with {'type': 'json'}; export default data;",
        "import './data.json' with {type: 'json'};",
        "import data from './data.json'; export default data;",
    ] {
        let parsed = parse(source, "entry.mjs").unwrap();
        let dependency = parsed.facts().dependencies().get(0).unwrap();
        assert_eq!(dependency.kind(), DependencyKind::Import);
        assert_eq!(
            dependency.json_attribute().is_some(),
            source.contains("with")
        );
        if let Some(span) = dependency.json_attribute() {
            assert!(source[span.start as usize..span.end as usize]
                .trim_start()
                .starts_with("with"));
            assert_eq!(span.start, dependency.specifier_span().end);
        }
        assert_eq!(
            &source[dependency.span().start as usize..dependency.span().end as usize],
            &source[..source.find(';').unwrap() + 1]
        );
    }
    for source in [
        "import data from './data.json' with {};",
        "import data from './data.json' with {type: 'css'};",
        "import data from './data.json' with {type: 'json', other: 'value'};",
        "import data from './data.json' with {'\\uD800': 'json'};",
        "import {value} from './data.json' with {type: 'json'};",
        "import * as data from './data.json' with {type: 'json'};",
        "export {default} from './data.json' with {type: 'json'};",
        "export * from './data.json' with {type: 'json'};",
        "import('./data.json', {with: {type: 'json'}});",
        "import source data from './data.json' with {type: 'json'};",
    ] {
        assert!(parse(source, "entry.mjs").is_err(), "{}", source);
    }
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
    let span = module.segments().get(0).unwrap().span;
    assert_eq!(module.body(), ModuleBody::CommonJs { span });
    assert_eq!(module.segments().len(), 1);
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        &source[source.find('\n').unwrap()..]
    );
    assert_eq!(
        module.facts().directives().get(0).unwrap().value(),
        "use strict"
    );
    assert_eq!(module.facts().dependencies().len(), 4);
    assert_eq!(
        module
            .facts()
            .dependencies()
            .get(0)
            .unwrap()
            .enclosing_function(),
        None
    );
    for dependency in module.facts().dependencies().slice(1..3) {
        let function = dependency.enclosing_function().unwrap();
        assert!(function.start <= dependency.span().start && function.end >= dependency.span().end);
    }
    assert_eq!(
        module
            .facts()
            .dependencies()
            .get(3)
            .unwrap()
            .enclosing_function(),
        None
    );
    assert!(module
        .facts()
        .references()
        .iter()
        .any(|r| r.commonjs() == Some(CommonJsGlobal::Module)));
    assert!(module
        .facts()
        .references()
        .iter()
        .any(|r| r.commonjs() == Some(CommonJsGlobal::Exports) && r.write()));
    assert_eq!(
        module.owned_segments().unwrap()[0]
            .module_deps
            .requires
            .len(),
        2
    );
    assert_eq!(
        module.owned_segments().unwrap()[0]
            .module_deps
            .dynamic_imports
            .len(),
        1
    );
    assert!(module.facts().exports().is_empty());
    assert_eq!(
        module.owned_facts().unwrap(),
        parse(source, "test.cjs").unwrap().owned_facts().unwrap()
    );
    assert!(module.facts().ecco().is_none());
    let mut facts = module.into_facts().into_owned().unwrap();
    *facts.ecco_declarations_hash_mut() = Default::default();
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
    assert_eq!(module.facts().dependencies().len(), 1);
    assert_eq!(
        module.facts().dependencies().get(0).unwrap().specifier(),
        "./real"
    );
    for name in ["require", "module", "exports"] {
        assert!(module
            .facts()
            .references()
            .iter()
            .any(|r| r.name() == name && r.binding().is_some() && r.commonjs().is_none()));
        assert!(module
            .facts()
            .references()
            .iter()
            .any(|r| r.name() == name && r.binding().is_none() && r.commonjs().is_some()));
    }
    let module = parse(
        "type require = string; interface module {x: number} type exports = number;\n\
         module.exports = require('./runtime'); exports.x = 1;",
        "types.cts",
    )
    .unwrap();
    assert_eq!(module.facts().dependencies().len(), 1);
    assert_eq!(
        module.facts().dependencies().get(0).unwrap().specifier(),
        "./runtime"
    );
    assert!(module
        .facts()
        .references()
        .iter()
        .all(|r| r.commonjs().is_some()));
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
        assert_eq!(module.segments().len(), 1);
        assert_eq!(module.segments().get(0).unwrap().span.start, 0);
        assert_eq!(
            module.segments().get(0).unwrap().span.end as usize,
            source.len()
        );
    }
    let module = parse(
        "const x: number = 1; module.exports = x; return;",
        "test.cts",
    )
    .unwrap();
    assert!(matches!(module.body(), ModuleBody::CommonJs { .. }));
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
        assert!(matches!(module.body(), ModuleBody::CommonJs { .. }));
        assert_eq!(module.segments().len(), 1);
    }
    let module = parse_module(
        &logger::StdioLogger::new(),
        "export const x = 1;",
        Path::new("file.mjs"),
        ModuleFormat::CommonJs,
    )
    .unwrap();
    assert_eq!(module.body(), ModuleBody::Esm);
}

#[test]
fn eval_identity_reads_remain_native_but_operations_are_rejected() {
    for path in ["primordials.mjs", "primordials.cjs"] {
        for source in [
            "const primordials = Object.freeze([Array, eval, Function]);",
            "const primordials = (() => [eval])();",
            "const identity = eval;",
            "const identity = {eval};",
            "const identity = flag ? eval : other;",
            "const identity = flag && eval;",
            "const identity = (other, eval);",
            "observe(eval);",
            "typeof eval;",
            "function read() { return eval; }",
            "const identity = (eval, other)();",
            "const identity = (eval ? first : second)();",
        ] {
            let parsed = parse(source, path).unwrap();
            let references: Vec<_> = parsed
                .facts()
                .references()
                .iter()
                .filter(|reference| reference.name() == "eval")
                .collect();
            assert_eq!(references.len(), 1, "{source}");
            assert!(references.iter().all(|reference| reference.read()
                && !reference.write()
                && reference.binding().is_none()
                && reference.commonjs().is_none()));
            assert!(parsed.facts().dependencies().is_empty());
        }
        for source in [
            "eval('local');",
            "(eval)('local');",
            "eval?.('local');",
            "(0, eval)('global');",
            "(flag ? eval : other)('code');",
            "(flag && eval)('code');",
            "eval.call(null, 'code');",
            "eval['apply'](null, ['code']);",
            "new eval('code');",
            "eval`code`;",
            "eval = other;",
            "eval++;",
            "({value: eval} = other);",
        ] {
            let error = format!("{:#}", parse(source, path).unwrap_err());
            // Strict ESM writes may fail semantic validation before the policy gate.
            assert!(error.contains(path), "{}: {}", source, error);
            if path.ends_with(".cjs") || (!source.contains(" = ") && !source.contains("++")) {
                assert!(
                    error.contains("unsupported unshadowed eval operation")
                        && error.contains("bytes "),
                    "{}: {}",
                    source,
                    error
                );
            }
        }
    }
    parse(
        "function read(eval) { return eval('ordinary call'); }",
        "shadow.cjs",
    )
    .unwrap();
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
        ("const load = require; load('./x');", "require-indirect-use"),
        ("require.resolve('./x');", "require-indirect-use"),
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
    assert_eq!(
        module.facts().dependencies().get(0).unwrap().specifier(),
        "./x"
    );
    assert_eq!(
        module.facts().dependencies().get(1).unwrap().specifier(),
        "./y"
    );
    assert!(module
        .facts()
        .references()
        .iter()
        .any(|r| r.commonjs() == Some(CommonJsGlobal::Module) && r.write()));
}

#[test]
fn commonjs_guarded_module_detectors_preserve_wrapper_reference_facts() {
    for detector in [
        "typeof module == 'object' && module && !module.nodeType && module",
        "'object' == typeof module && module && !module.nodeType && module",
        "typeof (module) === 'object' && (module) && !(module).nodeType && (module)",
        "i && 'object' === typeof module && module && !module.nodeType && module",
    ] {
        let source = format!(
            "function helper() {{
                var i = typeof exports == 'object' && exports && !exports.nodeType && exports;
                var o = {detector};
                var a = o && o.exports === i && host.process;
                try {{ return o && o.require && o.require('util').types ||
                    a && a.binding && a.binding('util'); }} catch (e) {{}}
             }}
             module.exports = helper;"
        );
        let module = parse(&source, "detector.cjs").unwrap();
        assert!(matches!(module.body(), ModuleBody::CommonJs { .. }));
        assert_eq!(module.segments().len(), 1);
        assert!(module.facts().dependencies().is_empty());
        let references: Vec<_> = module
            .facts()
            .references()
            .iter()
            .filter(|r| r.name() == "module")
            .collect();
        assert_eq!(references.len(), 5);
        assert!(references.iter().all(|r| r.binding().is_none()
            && r.commonjs() == Some(CommonJsGlobal::Module)
            && !r.write()));
    }
    let repeated = "var i, o;
        i = 'object' == typeof exports && exports && !exports.nodeType && exports,
        o = i && 'object' == typeof module && module && !module.nodeType && module;";
    let source = format!(
        "(function() {{ {} }})(); (function() {{ {} }})(); (function() {{ {} }})();",
        repeated, repeated, repeated
    );
    let repeated = parse(&source, "repeated.cjs").unwrap();
    assert_eq!(
        repeated
            .facts()
            .references()
            .iter()
            .filter(|r| r.commonjs() == Some(CommonJsGlobal::Module))
            .count(),
        12
    );
    let shadow = parse(
        "function local(module) {
            return typeof module == 'object' && module && !module.nodeType && module;
         }
         module.exports = local;",
        "shadow.cjs",
    )
    .unwrap();
    assert_eq!(
        shadow
            .facts()
            .references()
            .iter()
            .filter(|r| r.name() == "module" && r.binding().is_some() && r.commonjs().is_none())
            .count(),
        4
    );
    parse("module.exports = typeof module;", "typeof.cjs").unwrap();
}

#[test]
fn commonjs_guarded_computed_module_probe_keeps_opaque_facts() {
    let source = r#"
        var enqueueTaskImpl;
        try {
            var requireString = ('require' + Math.random()).slice(0, 7);
            var nodeRequire = module && module[requireString];
            enqueueTaskImpl = nodeRequire.call(module, 'timers').setImmediate;
        } catch (error) {
            var channel = new MessageChannel();
            enqueueTaskImpl = function(task) { channel.port2.postMessage(task); };
        }
        module.exports = enqueueTaskImpl;
    "#;
    let parsed = parse(source, "computed.cjs").unwrap();
    assert!(matches!(parsed.body(), ModuleBody::CommonJs { .. }));
    assert_eq!(parsed.segments().len(), 1);
    assert!(parsed.facts().dependencies().is_empty());
    assert!(parsed.facts().policy_violations().is_empty());
    assert_eq!(
        parsed
            .facts()
            .references()
            .iter()
            .filter(|reference| reference.commonjs() == Some(CommonJsGlobal::Module))
            .count(),
        4
    );
    for detector in [
        "(module) && (module)[key]",
        "module && module['require']",
        "module && module[('require' + Math.random()).slice(0, 7)]",
    ] {
        parse(
            &format!(
                "const key = 'require'; const selected = ({detector}); selected.call((module), 'timers');"
            ),
            "computed.cjs",
        )
        .unwrap();
    }
    for key in ["key", "'exports'", "'ex' + 'ports'"] {
        parse(
            &format!("const selected = module && module[{key}]; consume(selected);"),
            "computed.cjs",
        )
        .unwrap();
    }
    let shadow = parse(
        "function local(module) {
            const nodeRequire = module && module[key];
            return nodeRequire.call(module, 'timers');
         } module.exports = local;",
        "shadow.cjs",
    )
    .unwrap();
    assert_eq!(
        shadow
            .facts()
            .references()
            .iter()
            .filter(|reference| reference.name() == "module"
                && reference.binding().is_some()
                && reference.commonjs().is_none())
            .count(),
        3
    );
}

#[test]
fn commonjs_guarded_computed_module_probe_admission_stays_bounded() {
    for source in [
        "const r = module[key];",
        "const r = module && module.require;",
        "const r = module || module[key];",
        "const r = other && module[key];",
        "const r = module && other[key];",
        "const r = module && module?.[key];",
        "const r = module && module[module];",
        "module && (module[key] = 1);",
        "module && module[key]++;",
        "module && delete module[key];",
        "(module && module[key])();",
        "new (module && module[key])();",
        "(module && module[key])`tag`;",
        "(module && module[key]).call(module, 'timers');",
        "(module && module[key]).x = 1;",
        "delete (module && module[key]).x;",
        "const r = module && module[key]; consume(module);",
        "const r = module && module[key]; r(module);",
        "const r = module && module[key]; r.apply(module, []);",
        "const r = module && module[key]; r.call({}, module);",
        "const r = module && module[key]; r.call(module, module);",
        "const r = module && module[key]; r?.call(module, 'timers');",
        "const r = module && module[key]; r.call?.(module, 'timers');",
        "const r = module && module[key]; other.call(module, 'timers');",
        "const r = module && module['exports']; r.call = consume; r.call(module);",
        "exports.call = consume; const r = module && module['exports']; r.call(module);",
        "const r = module && module['exports']; exports.call = consume; r.call(module);",
        "module.exports.call = consume; const r = module && module['exports']; r.call(module);",
        "const key = 'exports'; exports.call = consume; const r = module && module[key]; r.call(module);",
        "const key = 'ex' + 'ports'; exports.call = consume; const r = module && module[key]; r.call(module);",
        "const key = ('exports' + Math.random()).slice(0, 7); exports.call = consume; const r = module && module[key]; r.call(module);",
        "exports.call = consume; const r = module && module[unknown]; r.call(module);",
        "const r = module && module['require']; r.call = consume; r.call(module);",
        "const r = module && module['require']; r['call'] = consume; r.call(module);",
        "const r = module && module['require']; r.call++; r.call(module);",
        "const r = module && module['require']; delete r.call; r.call(module);",
        "const r = module && module['require']; [r.call] = [consume]; r.call(module);",
        "const r = module && module['require']; consume(r); r.call(module);",
        "const r = module && module['require']; Object.defineProperty(r, 'call', {value: consume}); r.call(module);",
        "const r = module && module['require']; const escaped = r; escaped.call = consume; r.call(module);",
        "const r = module && module['require']; const escaped = {r}; escaped.r.call = consume; r.call(module);",
        "const r = module && module['require']; r.changeCall(); r.call(module);",
        "const r = module && module['require']; r.call(module, consume(r));",
        "const r = module && module['require']; function mutate() { r.call = consume; } mutate(); r.call(module);",
        "let r = module && module['require']; r = other; r.call(module, 'timers');",
        "let r = module && module['require']; function replace() { r = other; } r.call(module, 'timers');",
        "var r = module && module['require']; var r; r.call(module, 'timers');",
        "const r = module && module['require']; function local(r) { r.call(module, 'timers'); }",
        "const r = module && module[key]; r.call(module);",
        "let key = 'require'; key = 'exports'; const r = module && module[key]; r.call(module);",
        "var key = 'require'; var key; const r = module && module[key]; r.call(module);",
        "const key = 'require'; function local(key) { const r = module && module[key]; r.call(module); }",
        "const key = 'require'; const other = key; const r = module && module[other]; r.call(module);",
        "const key = ('require' + Math.random()).slice(1, 7); const r = module && module[key]; r.call(module);",
        "const key = ('require' + Math.random()).slice(0, 8); const r = module && module[key]; r.call(module);",
        "function local(module) { return module && module[key]; } other.call(module, 'timers');",
        "function local(module) { var r = module && module[key]; } r.call(module, 'timers');",
        "const r = module && module[key]; module[other];",
    ] {
        let error = format!("{:#}", parse(source, "bad.cjs").unwrap_err());
        assert!(error.contains("module access"), "{}: {}", source, error);
    }
}

#[test]
fn commonjs_module_null_checks_preserve_wrapper_reference_facts() {
    for operator in ["==", "!=", "===", "!=="] {
        for comparison in [
            format!("(module) {operator} (null)"),
            format!("(null) {operator} (module)"),
        ] {
            let source = format!(
                "(function() {{
                    if (typeof module !== 'undefined' && {comparison}
                        && typeof exports !== 'undefined' && module.exports === exports) {{
                        module.exports = 7;
                    }}
                }}());"
            );
            let parsed = parse(&source, "null-check.cjs").unwrap();
            assert!(matches!(parsed.body(), ModuleBody::CommonJs { .. }));
            assert_eq!(parsed.segments().len(), 1);
            assert!(parsed.facts().dependencies().is_empty());
            let references: Vec<_> = parsed
                .facts()
                .references()
                .iter()
                .filter(|reference| reference.name() == "module")
                .collect();
            assert_eq!(references.len(), 4);
            assert!(references
                .iter()
                .all(|reference| reference.binding().is_none()
                    && reference.commonjs() == Some(CommonJsGlobal::Module)
                    && !reference.write()));
        }
    }
    let parsed = parse(
        "function local(module) { return module !== null; } module.exports = local;",
        "shadow.cjs",
    )
    .unwrap();
    let shadow = parsed
        .facts()
        .references()
        .iter()
        .find(|reference| reference.name() == "module" && reference.binding().is_some())
        .unwrap();
    assert!(shadow.commonjs().is_none());
}

#[test]
fn commonjs_module_detector_admission_stays_bounded() {
    for source in [
        "module.require('util');",
        "module['require']('util');",
        "module[name]('util');",
        "const m = module; m.require('util');",
        "consume(module);",
        "module();",
        "new module();",
        "module`tag`;",
        "module.nodeType;",
        "delete module.nodeType;",
        "module.nodeType = 1;",
        "module.nodeType++;",
        "typeof module == 'object' && module;",
        "typeof module == 'object' && module && module.nodeType && module;",
        "typeof module == 'object' || module && !module.nodeType && module;",
        "typeof module == 'function' && module && !module.nodeType && module;",
        "typeof other == 'object' && module && !module.nodeType && module;",
        "typeof module == 'object' && module && !module[name] && module;",
        "typeof module == 'object' && module && !module?.nodeType && module;",
        "(typeof module == 'object' && module && !module.nodeType && module)();",
        "new (typeof module == 'object' && module && !module.nodeType && module)();",
        "(typeof module == 'object' && module && !module.nodeType && module)`tag`;",
        "(typeof module == 'object' && module && !module.nodeType && module).require('util');",
        "delete (typeof module == 'object' && module && !module.nodeType && module).x;",
        "(typeof module == 'object' && module && !module.nodeType && module).x = 1;",
        "module === undefined;",
        "module === other;",
        "module < null;",
        "module + null;",
        "module.nodeType !== null;",
        "module.require !== null;",
        "module[name] !== null;",
        "module() !== null;",
        "new module() !== null;",
        "module`tag` !== null;",
        "(module !== null) && consume(module);",
        "module !== null && module.require('util');",
    ] {
        let error = format!("{:#}", parse(source, "bad.cjs").unwrap_err());
        assert!(
            error.contains("module access") && error.contains("bytes "),
            "{}: {}",
            source,
            error
        );
    }
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
fn esm_environment_probes_are_free_reads_not_commonjs_bindings() {
    let source = r#"
var freeExports = typeof exports == 'object' && exports && !exports.nodeType && exports;
var freeModule = freeExports && typeof module == 'object' && module && !module.nodeType && module;
var moduleExports = freeModule && freeModule.exports === freeExports;
var nodeUtil = freeModule && freeModule.require && freeModule.require('util');
export const detected = moduleExports;
export function guarded() {
    if (typeof module !== 'undefined') return module['exports'];
    return typeof exports === 'undefined' ? undefined : exports.nodeType;
}
export const loaderType = typeof (require);
"#;
    let parsed = parse(source, "probes.mjs").unwrap();
    assert_eq!(parsed.body(), ModuleBody::Esm);
    assert!(!parsed.facts().commonjs());
    assert!(parsed.facts().dependencies().is_empty());
    for name in ["exports", "module", "require"] {
        let references: Vec<_> = parsed
            .facts()
            .references()
            .iter()
            .filter(|r| r.name() == name)
            .collect();
        assert!(!references.is_empty());
        assert!(references
            .iter()
            .all(|r| r.binding().is_none() && r.commonjs().is_none() && !r.write()));
    }
}

#[test]
fn esm_environment_probes_preserve_shadows_and_unprotected_read_semantics() {
    for source in [
        "export function f(module, exports, require) { module.exports = exports; return require('local'); }",
        "export function f() { return typeof exports; let exports; }",
        "export const value = module.exports;",
        "export const value = typeof exports['nodeType'];",
        "export function f() { if (typeof module === 'object') return module.exports; }",
        "export const value = (() => typeof module)();",
        "export const value = (function () { return typeof exports; })();",
        "const target = {}; target[typeof module] = 1; export default target;",
        "export const value = table[typeof exports]();",
        "export const value = wrap(typeof require)();",
        "export const value = (typeof module === 'object' && module ? nodeImpl : browserImpl)();",
        "export const value = (module, exports, browserImpl)();",
        "export const value = (module === null || browserImpl)();",
        "export const value = new (module ? NodeImpl : BrowserImpl)();",
        "export const value = (exports ? nodeTag : browserTag)`value`;",
    ] {
        parse(source, "probes.mjs").unwrap();
    }
}

#[test]
fn esm_environment_probes_select_native_host_loaders_without_dependencies() {
    for selection in [
        "typeof __webpack_require__ === 'function' ? __non_webpack_require__ : require",
        "typeof __webpack_require__ !== 'function' ? (require) : __non_webpack_require__",
        "outer ? (inner ? require : other) : fallback",
    ] {
        let source = format!(
            "export function load() {{
                const requireFunc = {selection};
                return requireFunc('host-only');
            }}"
        );
        let parsed = parse(&source, "host-loader.mjs").unwrap();
        assert_eq!(parsed.body(), ModuleBody::Esm);
        assert!(!parsed.facts().commonjs());
        assert!(parsed.facts().dependencies().is_empty());
        assert!(parsed
            .facts()
            .references()
            .iter()
            .filter(|reference| reference.name() == "require")
            .all(|reference| reference.binding().is_none()
                && reference.commonjs().is_none()
                && !reference.write()));
    }
    parse(
        "export function load(require) {
            const host = flag ? require : fallback; return host(name);
        }",
        "shadow.mjs",
    )
    .unwrap();
}

#[test]
fn esm_environment_probes_do_not_enable_wrapper_operations() {
    for source in [
        "module.exports = 1;",
        "exports.x++;",
        "delete exports.x;",
        "({x: module.exports} = value);",
        "for (exports.x of values) {}",
        "module['exports'].x = 1;",
        "typeof module !== 'undefined' && (module.exports = 1);",
        "module.require('./x');",
        "module?.require?.('./x');",
        "(0, module.require)('./x');",
        "new exports.Factory();",
        "exports.tag`value`;",
        "const load = require;",
        "const load = flag ? require.resolve : fallback;",
        "const load = flag ? (require = loader) : fallback;",
        "const load = flag ? require++ : fallback;",
        "(flag ? require : fallback)('./x');",
        "new (flag ? require : fallback)('./x');",
        "(flag ? require : fallback)`./x`;",
        "(flag ? require : fallback).resolve('./x');",
        "(flag ? require : fallback).cache = {};",
        "typeof require.resolve;",
        "(module ? module.require : browserImpl)('./x');",
        "(exports ? browserImpl : exports.run)();",
        "(module, exports.run)();",
        "(module.require('./x'), browserImpl)();",
        "(module.exports = 1, browserImpl)();",
        "(module.require || browserImpl)();",
        "(browserImpl && module.require)();",
        "(module.require ?? browserImpl)();",
        "(module.require && browserImpl)();",
        "new (module ? exports.Factory : BrowserImpl)();",
        "(module ? exports.tag : browserTag)`value`;",
    ] {
        let error = format!("{:#}", parse(source, "probes.mjs").unwrap_err());
        assert!(
            error.contains("mixed ESM/CommonJS") && error.contains("bytes "),
            "{}: {}",
            source,
            error
        );
    }
}

#[test]
fn commonjs_require_detectors_preserve_native_reads_without_dependencies() {
    for guard in [
        "'function' == typeof require && require",
        "typeof require === 'function' && require",
        "('function' === (typeof (require))) && (require)",
        "(typeof require) == ('function') && require",
    ] {
        let source = format!(
            "var fallback = {guard};
             module.exports = function(name) {{
                 var current = {guard};
                 return current ? current(name, true) : fallback ? fallback(name, true) : 'missing';
             }};"
        );
        let parsed = parse(&source, "browserify.cjs").unwrap();
        assert!(parsed.facts().commonjs());
        assert!(parsed.facts().dependencies().is_empty());
        let references: Vec<_> = parsed
            .facts()
            .references()
            .iter()
            .filter(|reference| reference.name() == "require")
            .collect();
        assert_eq!(references.len(), 4);
        assert!(references.iter().all(|reference| reference.read()
            && !reference.write()
            && reference.binding().is_none()
            && reference.commonjs() == Some(CommonJsGlobal::Require)));
    }
    let parsed = parse(
        "typeof require; var host = typeof require === 'function' && require;
         module.exports = () => require('./real.cjs');",
        "browserify.cjs",
    )
    .unwrap();
    assert_eq!(parsed.facts().dependencies().len(), 1);
    assert_eq!(
        parsed.facts().dependencies().get(0).unwrap().specifier(),
        "./real.cjs"
    );
    let parsed = parse(
        "function shadow(require) {
            var host = typeof require === 'function' && require;
            return host('local');
         } module.exports = shadow;",
        "browserify.cjs",
    )
    .unwrap();
    assert!(parsed.facts().dependencies().is_empty());
    assert!(parsed
        .facts()
        .references()
        .iter()
        .filter(|reference| reference.name() == "require")
        .all(|reference| reference.binding().is_some() && reference.commonjs().is_none()));
}

#[test]
fn commonjs_require_detector_admission_stays_bounded() {
    for source in [
        "var loader = require;",
        "var loader = flag && require;",
        "var loader = typeof require === 'object' && require;",
        "var loader = typeof require !== 'function' && require;",
        "var loader = typeof require === 'function' || require;",
        "var loader = typeof other === 'function' && require;",
        "var loader = typeof require === 'function' && (require, other);",
        "var loader = typeof require === 'function' && require.resolve;",
        "var loader = typeof require === 'function' && (require = other);",
        "var loader = typeof require === 'function' && require++;",
        "var loader = typeof require === 'function' ? require : other;",
        "(typeof require === 'function' && require)('indirect');",
        "new (typeof require === 'function' && require)('indirect');",
        "(typeof require === 'function' && require)`indirect`;",
        "(typeof require === 'function' && require).resolve('indirect');",
        "(typeof require === 'function' && require)['resolve']('indirect');",
        "typeof require.resolve;",
    ] {
        let error = format!("{:#}", parse(source, "unsupported.cjs").unwrap_err());
        assert!(
            (error.contains("require-indirect-use") || error.contains("require-reassignment"))
                && error.contains("bytes "),
            "{}: {}",
            source,
            error
        );
    }
    assert!(parse(
        "export const loader = typeof require === 'function' && require;",
        "unsupported.mjs"
    )
    .is_err());
}

#[test]
fn mixed_modules_are_errors_not_partial_records() {
    for (path, source) in [
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
fn esm_direct_require_preserves_format_occurrences_and_shadows() {
    let source = "const rawChatLoaderVersion = require('../package.json').version; \
        export const chatLoaderVersion = String(rawChatLoaderVersion); \
        export const load = () => (require)(('../package.json')); \
        function shadow(require) { return require(path); }";
    let parsed = parse(source, "runtimeCreator.js").unwrap();
    assert_eq!(parsed.body(), ModuleBody::Esm);
    let facts = parsed.facts();
    assert!(!facts.commonjs());
    assert!(facts.ecco().is_some());
    assert_eq!(facts.dependencies().len(), 2);
    for dependency in facts.dependencies().iter() {
        assert_eq!(dependency.kind(), DependencyKind::Require);
        assert_eq!(dependency.specifier(), "../package.json");
        assert!(dependency.pattern().is_none());
    }
    assert!(facts
        .dependencies()
        .get(0)
        .unwrap()
        .enclosing_function()
        .is_none());
    assert!(facts
        .dependencies()
        .get(1)
        .unwrap()
        .enclosing_function()
        .is_some());
    assert!(facts
        .references()
        .iter()
        .all(|reference| reference.commonjs().is_none()));
    assert!(facts
        .references()
        .iter()
        .any(|reference| reference.name() == "require" && reference.binding().is_some()));
    parse(
        "typeof require === 'function' && require('./x');",
        "probe.mjs",
    )
    .unwrap();
}

#[test]
fn esm_require_rejects_unsupported_loader_forms_and_contexts() {
    for source in [
        "require = loader;",
        "const load = require; load('./x');",
        "require.resolve('./x');",
        "(0, require)('./x');",
        "new require('./x');",
        "require?.('./x');",
        "require('./x', effect());",
        "require(path);",
        "require(`./parts/${name}.js`);",
        "require('./parts/' + name);",
    ] {
        let error = format!("{:#}", parse(source, "bad.mjs").unwrap_err());
        assert!(
            error.contains("bad.mjs") && error.contains("bytes "),
            "{}: {}",
            source,
            error
        );
    }
}

#[test]
fn esm_top_level_await_facts_exclude_function_bodies_and_cjs_remains_sync() {
    let source = "await (await 1); for await (const x of stream) { await x; } async function f() { await 2; for await (const y of stream) {} } const g = async () => await 3;";
    let parsed = parse(source, "async.mjs").unwrap();
    let sites: Vec<_> = parsed
        .facts()
        .top_level_await()
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
        .facts()
        .top_level_await()
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
            "import x from './x' with {type: 'css'};",
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
    let binding = module.facts().bindings().get(0).unwrap();
    assert_eq!(
        &source[binding.declarations().get(0).unwrap().start as usize
            ..binding.declarations().get(0).unwrap().end as usize],
        "café"
    );
    for reference in module.facts().references() {
        assert_eq!(
            &source[reference.span().start as usize..reference.span().end as usize],
            reference.name()
        );
    }
    let dependency = module.facts().dependencies().get(0).unwrap();
    assert_eq!(
        &source
            [dependency.specifier_span().start as usize..dependency.specifier_span().end as usize],
        "'./dépendance'"
    );
    assert_eq!(
        &source[dependency.span().start as usize..dependency.span().end as usize],
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
    assert_eq!(module.facts().dependencies().len(), 5);
    assert_eq!(
        module
            .facts()
            .dependencies()
            .iter()
            .map(|d| d.specifier())
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
        module.facts().dependencies().get(0).unwrap().kind(),
        DependencyKind::ImportType
    );
    assert!(module.facts().dependencies().get(0).unwrap().type_only());
    assert_eq!(
        module.owned_segments().unwrap()[0]
            .module_deps
            .requires
            .len(),
        2
    );
    assert_eq!(
        module.owned_segments().unwrap()[0]
            .module_deps
            .dynamic_imports
            .len(),
        2
    );
}
