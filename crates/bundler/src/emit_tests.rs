use super::*;
use source_graph::GraphNode;
use std::path::Path;
use std::process::{Command, Output};
use test_tmpdir::TmpDir;

fn emit(source: &str, namespace: bool) -> String {
    let graph = SourceGraph::from_owned([crate::tests::input("input.mjs", source)]);
    let roots = if namespace {
        vec![GraphNode::Module(0)]
    } else {
        vec![]
    };
    let policy = EffectPolicy::default();
    let retention = crate::retain_with_policy(&graph, &[0], &roots, &policy).unwrap();
    single_module(&graph, &retention, &policy, 0, source).unwrap()
}

fn node(source: &str, observer: &str) -> Output {
    let dir = TmpDir::new();
    dir.write_batch(&std::collections::HashMap::from([
        ("input.mjs".into(), source),
        ("run.mjs".into(), observer),
    ]))
    .unwrap();
    Command::new("node")
        .arg(dir.root_join("run.mjs"))
        .current_dir(dir.root())
        .output()
        .unwrap()
}

fn compare(source: &str, observer: &str, namespace: bool) -> String {
    let baseline = node(source, observer);
    let output = emit(source, namespace);
    let bundled = node(&output, observer);
    assert_eq!(
        bundled.status.success(),
        baseline.status.success(),
        "emitted:\n{output}\nstderr:\n{}",
        String::from_utf8_lossy(&bundled.stderr)
    );
    assert_eq!(bundled.stdout, baseline.stdout, "emitted:\n{output}");
    if !baseline.status.success() {
        assert!(
            String::from_utf8_lossy(&bundled.stderr).contains("ReferenceError"),
            "{}",
            String::from_utf8_lossy(&bundled.stderr)
        );
        if String::from_utf8_lossy(&baseline.stderr).contains("before initialization") {
            assert!(String::from_utf8_lossy(&bundled.stderr).contains("before initialization"));
        }
    }
    output
}

#[test]
fn tree_shakes_real_code_and_preserves_shadowing_hoisting_and_order() {
    let source = r#"
#!/not-a-real-hashbang
"#;
    // A hashbang is only valid at byte zero.
    assert!(rewrite::validate(Path::new("bad.mjs"), source).is_err());
    let source = r#"#!/usr/bin/env node
// copyright é
'use strict';
function unused() { throw new Error("must disappear"); }
console.log(hoisted(), before);
var before = 3;
function hoisted() { return 7; }
let x = 1;
{ let x = 2; console.log(x); }
console.log(x, before);
//# sourceMappingURL=obsolete.map
"#;
    let output = compare(source, "import './input.mjs';", false);
    assert!(!output.contains("must disappear"));
    assert!(!output.contains("obsolete.map"));
    assert!(output.starts_with("#!/usr/bin/env node\n// copyright é\n'use strict';"));
    assert!(output.len() < source.len());
}

#[test]
fn forward_lexical_tdz_throws_and_live_export_aliases_stay_live() {
    compare(
        "console.log(later); let later = 1;",
        "import './input.mjs';",
        false,
    );
    compare(
        "export let value = 1; export {value as alias}; export function bump() { value++; } export default function named() { return value; }",
        "import * as ns from './input.mjs'; console.log(ns.value, ns.alias, ns.default()); ns.bump(); console.log(ns.value, ns.alias, ns.default());",
        true,
    );
    for declaration in [
        "export default 42;",
        "export default function() { return 42; }",
        "export default class { static value = 42; }",
        "const value = 42; export {value as default};",
    ] {
        compare(
            declaration,
            "import * as ns from './input.mjs'; console.log(typeof ns.default);",
            true,
        );
    }
}

#[test]
fn pure_recipes_remove_invocation_not_input_effects() {
    let source = r#"
let log = [];
const obj = { get method() { log.push('get'); return function() { log.push('WRONG'); }; } };
const iterable = { *[Symbol.iterator]() { log.push('iterate'); yield 1; } };
const side = () => (log.push('arg'), 2);
/*#__PURE__*/ obj.method(side(), ...iterable);
console.log(log.join(','));
"#;
    let output = emit(source, false);
    let actual = node(&output, "import './input.mjs';");
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        String::from_utf8(actual.stdout).unwrap(),
        "get,arg,iterate\n"
    );
    assert!(!output.contains("obj.method(side()"));
}

#[test]
fn optional_pure_guards_distinguish_chain_completion_from_undefined() {
    for (callee, expected) in [
        ("absent?.method", ""),
        ("present?.method", "get,arg"),
        ("present.method", "get,arg"),
        ("present.method?.", "get"),
        ("present?.method?.", "get"),
        ("present?.[key()]", "key,get,arg"),
        ("(absent?.method)", "arg"),
    ] {
        let optional = callee.ends_with("?.");
        let callee = callee.trim_end_matches("?.");
        let source = format!(
            r#"
let log = [];
const absent = null;
const present = {{get method() {{ log.push('get'); return undefined; }} }};
function key() {{ log.push('key'); return 'method'; }}
function arg() {{ log.push('arg'); }}
/*#__PURE__*/ {callee}{}(arg());
console.log(log.join(','));
"#,
            if optional { "?." } else { "" }
        );
        let output = emit(&source, false);
        let actual = node(&output, "import './input.mjs';");
        assert!(
            actual.status.success(),
            "{callee}: {}\n{output}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(
            String::from_utf8(actual.stdout).unwrap(),
            format!("{expected}\n"),
            "{callee}\n{output}"
        );
    }
}

#[test]
fn late_binding_demand_restores_pure_results_and_nested_recipes_compose() {
    let source = "function make() { console.log('called'); return 42; } export const value = /*#__PURE__*/ make();";
    compare(
        source,
        "import {value} from './input.mjs'; console.log(value);",
        true,
    );
    let source = r#"
let log = [];
const obj = { method() { log.push('WRONG'); } };
function side() { log.push('arg'); }
/*#__PURE__*/ obj.method((/*#__PURE__*/ obj.method(side()), 1));
console.log(log.join(','));
"#;
    let output = emit(source, false);
    let actual = node(&output, "import './input.mjs';");
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(String::from_utf8(actual.stdout).unwrap(), "arg\n");
}

#[test]
fn unsupported_modes_and_stale_sources_fail_without_artifacts() {
    let policy = EffectPolicy::default();
    for (path, source) in [
        ("input.ts", "export const x: number = 1;"),
        ("input.jsx", "export const x = 1;"),
        ("input.cjs", "module.exports = 1;"),
    ] {
        let graph = SourceGraph::from_owned([crate::tests::input(path, source)]);
        let retention = crate::retain(&graph, &[0], &[]).unwrap();
        assert!(single_module(&graph, &retention, &policy, 0, source)
            .unwrap_err()
            .to_string()
            .contains("emission"));
    }
    let graph = SourceGraph::from_owned([crate::tests::input("input.mjs", "console.log(1);")]);
    let retention = crate::retain(&graph, &[0], &[]).unwrap();
    assert!(single_module(&graph, &retention, &policy, 0, "throw 1;").is_err());
    assert!(single_module(
        &graph,
        &retention,
        &policy.clone().with_pure_annotations(false),
        0,
        "console.log(1);"
    )
    .is_err());
    for source in ["console.log(import.meta.url);", "const x = <div/>;"] {
        assert!(ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new("input.mjs"),
            ModuleFormat::Esm
        )
        .is_err());
    }

    let source = "const obj = {}; function side() {} /*#__PURE__*/ obj.method?.().field(side());";
    let graph = SourceGraph::from_owned([crate::tests::input("input.mjs", source)]);
    let retention = crate::retain(&graph, &[0], &[]).unwrap();
    let error = format!(
        "{:#}",
        single_module(&graph, &retention, &policy, 0, source).unwrap_err()
    );
    assert!(
        error.contains("input.mjs") && error.contains("bytes") && error.contains("optional"),
        "{error}"
    );
}

#[test]
fn single_module_preserves_top_level_suspension() {
    let source = "console.log('before'); await Promise.resolve(); console.log('after');";
    let output = emit(source, false);
    let actual = node(&output, "await import('./input.mjs'); console.log('done');");
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"before\nafter\ndone\n");
}
