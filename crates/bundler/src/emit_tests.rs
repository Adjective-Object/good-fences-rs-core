use super::*;
use source_graph::GraphNode;
use std::path::Path;
use std::process::{Command, Output};
use test_tmpdir::TmpDir;

fn rendered(code: Code) -> String {
    code.render(
        Path::new("input.mjs"),
        Path::new("input.mjs"),
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn cached_operations_compose_names_shorthand_dependencies_and_nested_pure() {
    use ast_segmenter::emission::OperationKind;
    let source = "let value = 3; /*#__PURE__*/ remove(({value}), (/*#__PURE__*/ inner(value), 1), import(/*# sourceURL=obsolete */'./original.mjs'));";
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("input.mjs"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let segment = &parsed.segments[1];
    let rewrites = crate::effects::summarize(&parsed.facts, segment.span, false).rewrites;
    assert_eq!(rewrites.len(), 2);
    let recipe = &parsed.facts.emission.segments[1];
    let mut renderer = CachedRenderer::new(source, &parsed.facts, recipe, &rewrites);
    for (index, operation) in recipe.operations.iter().enumerate() {
        if let OperationKind::Reference {
            reference,
            shorthand,
        } = operation.kind
        {
            let reference = &parsed.facts.references[reference as usize];
            if reference.name == "value" {
                renderer.replace(
                    index as u32,
                    rewrite::shorthand_replacement(source, reference.span, "renamed", shorthand)
                        .unwrap()
                        .into(),
                );
            }
        }
    }
    let dependency = &parsed.facts.dependencies[0];
    renderer.dependency(dependency, "'./resolved.mjs'".into(), false);
    let count = rewrite::output_parse_count();
    let output = rendered(renderer.render(segment.span).unwrap());
    assert_eq!(rewrite::output_parse_count(), count);
    assert!(output.contains("{value: renamed}"), "{output}");
    assert!(output.contains("((inner),(renamed),void 0)"), "{output}");
    assert!(output.contains("import('./resolved.mjs')"), "{output}");
    assert!(!output.contains("obsolete"), "{output}");
    assert!(!output.contains("remove("), "{output}");
    rewrite::validate(Path::new("output.mjs"), &output).unwrap();
}

#[test]
fn original_coordinate_slots_reject_invalid_utf8_and_escaping_edits() {
    use crate::assembly::Edit;
    for (range, edits) in [
        (Span::new(1, 2), vec![]),
        (
            Span::new(2, 4),
            vec![Edit {
                span: Span::new(0, 2),
                replacement: "".into(),
            }],
        ),
        (
            Span::new(0, 4),
            vec![Edit {
                span: Span::new(2, 5),
                replacement: "".into(),
            }],
        ),
    ] {
        assert!(crate::assembly::apply_range("é12", range, &edits).is_err());
    }
}

#[test]
fn cached_optional_chain_slots_compose_without_fragment_parses() {
    use ast_segmenter::emission::OperationKind;
    let source = r#"
const log = [];
const holder = { make(x) { log.push('make'); return { get method() { log.push('get'); return undefined; } }; } };
function key() { log.push('key'); return 'make'; }
function side() { log.push('WRONG'); }
function arg() { log.push('arg'); return 1; }
/*#__PURE__*/ holder?.[(/*#__PURE__*/ side(), key())](arg()).method?.(arg());
console.log(log.join(','));
"#;
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("input.mjs"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let binding = parsed
        .facts
        .bindings
        .iter()
        .find(|binding| binding.name == "holder")
        .unwrap()
        .id;
    let before = rewrite::output_parse_count();
    let mut output = Code::default();
    for (segment, recipe) in parsed.segments.iter().zip(&parsed.facts.emission.segments) {
        let rewrites = crate::effects::summarize(&parsed.facts, segment.span, false).rewrites;
        let mut renderer = CachedRenderer::new(source, &parsed.facts, recipe, &rewrites);
        for (index, operation) in recipe.operations.iter().enumerate() {
            let id = match operation.kind {
                OperationKind::Binding { binding, .. } => Some(binding),
                OperationKind::Reference { reference, .. } => {
                    parsed.facts.references[reference as usize].binding
                }
                _ => None,
            };
            if id == Some(binding) {
                renderer.replace(index as u32, "renamed".into());
            }
        }
        output.push(renderer.render(segment.span).unwrap());
        output.push_str("\n;\n");
    }
    assert_eq!(rewrite::output_parse_count(), before);
    let output = rendered(output);
    assert!(output.contains("(renamed)"), "{output}");
    let actual = node(&output, "import './input.mjs';");
    assert!(
        actual.status.success(),
        "{}\n{output}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        String::from_utf8(actual.stdout).unwrap(),
        "key,arg,make,get\n"
    );
}

#[test]
fn cached_await_guard_selection_does_not_parse_callees() {
    for invocation in [
        "globalThis.run?.(await Promise.resolve(1))",
        "globalThis?.[await Promise.resolve('run')](1)",
        "(await Promise.resolve(globalThis))?.run(1)",
        "(globalThis?.run)(await Promise.resolve(1))",
    ] {
        let source = format!("/*#__PURE__*/ {invocation};");
        let parsed = ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            &source,
            Path::new("input.mjs"),
            ModuleFormat::Esm,
        )
        .unwrap();
        let segment = &parsed.segments[0];
        let rewrites = crate::effects::summarize(&parsed.facts, segment.span, false).rewrites;
        assert_eq!(rewrites.len(), 1);
        let mut renderer = CachedRenderer::new(
            &source,
            &parsed.facts,
            &parsed.facts.emission.segments[0],
            &rewrites,
        );
        let before = rewrite::output_parse_count();
        let output = rendered(renderer.render(segment.span).unwrap());
        assert_eq!(rewrite::output_parse_count(), before);
        assert_eq!(
            output.contains(invocation),
            !invocation.starts_with('('),
            "{output}"
        );
    }
}

#[test]
fn typed_dynamic_request_survives_nested_optional_pure_templates() {
    use crate::assembly::RequestKind;
    let source =
        "/*#__PURE__*/ receiver?.[key()](import('./original.mjs'), import('./original.mjs'));";
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("input.mjs"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let segment = &parsed.segments[0];
    let rewrites = crate::effects::summarize(&parsed.facts, segment.span, false).rewrites;
    let mut renderer = CachedRenderer::new(
        source,
        &parsed.facts,
        &parsed.facts.emission.segments[0],
        &rewrites,
    );
    renderer.dependency(
        &parsed.facts.dependencies[0],
        Code::request("target.mjs".into(), RequestKind::DynamicImport),
        false,
    );
    renderer.dependency(
        &parsed.facts.dependencies[1],
        Code::request("second.mjs".into(), RequestKind::DynamicImport),
        false,
    );
    let before = rewrite::output_parse_count();
    let code = renderer.render(segment.span).unwrap();
    assert_eq!(rewrite::output_parse_count(), before);
    assert!(code
        .render(
            Path::new("input.mjs"),
            Path::new("input.mjs"),
            &Default::default()
        )
        .is_err());
    let output = code
        .render(
            Path::new("input.mjs"),
            Path::new("nested/entry.mjs"),
            &std::collections::BTreeMap::from([
                ("target.mjs".into(), "chunks/a #é.mjs".into()),
                ("second.mjs".into(), "other/second.mjs".into()),
            ]),
        )
        .unwrap();
    assert!(
        output.contains("import(\"../chunks/a%20%23%C3%A9.mjs\")"),
        "{output}"
    );
    assert!(!output.contains("original.mjs"), "{output}");
    assert!(
        output.contains("import(\"../other/second.mjs\")"),
        "{output}"
    );
    rewrite::validate(Path::new("output.mjs"), &output).unwrap();
}

#[test]
fn checked_dependency_ids_address_static_import_and_reexport_wrappers() {
    for source in [
        "import { value } from './original.mjs';",
        "export { value } from './original.mjs';",
        "export * from './original.mjs';",
    ] {
        let parsed = ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new("input.mjs"),
            ModuleFormat::Esm,
        )
        .unwrap();
        let mut renderer = CachedRenderer::new(
            source,
            &parsed.facts,
            &parsed.facts.emission.segments[0],
            &[],
        );
        renderer.dependency(
            &parsed.facts.dependencies[0],
            "'./changed.mjs'".into(),
            false,
        );
        let output = rendered(renderer.render(parsed.segments[0].span).unwrap());
        assert_eq!(output, source.replace("./original.mjs", "./changed.mjs"));
        renderer.dependency(
            &parsed.facts.dependencies[0],
            "import './evaluate.mjs';".into(),
            true,
        );
        assert_eq!(
            rendered(renderer.render(parsed.segments[0].span).unwrap()),
            "import './evaluate.mjs';"
        );
    }
}

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

#[test]
fn source_provenance_rejects_changed_bytes_modes_and_unprovenanced_inputs() {
    let source = "export const value = 1;";
    let policy = EffectPolicy::default();
    let check = |graph: &SourceGraph, text: &str| {
        let retention = crate::retain_with_policy(graph, &[0], &[], &policy).unwrap();
        single_module(graph, &retention, &policy, 0, text)
    };
    let graph = SourceGraph::from_owned([crate::tests::input("input.mjs", source)]);
    assert!(check(&graph, source).is_ok());
    assert!(check(&graph, "export const value = 2;").is_err());

    let mut input = crate::tests::input("input.mjs", source);
    input.path = "input.cjs".into();
    let error = format!(
        "{:#}",
        check(&SourceGraph::from_owned([input]), source).unwrap_err()
    );
    assert!(error.contains("parser configuration mismatch"), "{error}");

    let mut input = crate::tests::input("input.mjs", source);
    input.provenance = None;
    let error = check(&SourceGraph::from_owned([input]), source).unwrap_err();
    assert!(error
        .to_string()
        .contains("missing source extraction provenance"));

    let mut input = crate::tests::input("input.mjs", source);
    input.facts.emission.segments[0].connector_only = true;
    assert!(check(&SourceGraph::from_owned([input]), source).is_err());
}

#[test]
fn prepared_css_emission_uses_sealed_mode_and_rejects_forged_provenance() {
    for (input_path, parser_path, source) in [
        (
            "style.css",
            "style.css.cjs",
            "module.exports = './assets/style.css';",
        ),
        (
            "style.css",
            "style.css.mjs",
            "export const button = '_button';",
        ),
        (
            "style.custom",
            "style.css.mjs",
            "const local = '_button'; export {local as 'class'};",
        ),
        (
            "style.module.cjs",
            "style.css.mjs",
            "export const button = '_button';",
        ),
        (
            "style.module.ts",
            "style.css.mjs",
            "export const button = '_button';",
        ),
        (
            "style.module.json",
            "style.css.mjs",
            "export const button = '_button';",
        ),
    ] {
        let input = || {
            let parsed = ast_segmenter::parse_module(
                &logger::StdioLogger::new(),
                source,
                Path::new(parser_path),
                ModuleFormat::Esm,
            )
            .unwrap();
            source_graph::OwnedSourceFileInput::from_prepared_resource(
                input_path.into(),
                parsed,
                Default::default(),
            )
        };
        let before = ast_segmenter::module_parse_count();
        let graph = SourceGraph::from_owned([input()]);
        let after_extraction = ast_segmenter::module_parse_count();
        assert_eq!(after_extraction, before + 1);
        let parsed = checked_source(&graph, 0, source).unwrap();
        assert_eq!(parsed.provenance(), graph.file_provenance(0).unwrap());
        assert!(checked_source(&graph, 0, &format!("{source}\n")).is_err());
        assert_eq!(ast_segmenter::module_parse_count(), after_extraction);

        let mut missing = input();
        missing.provenance = None;
        assert!(checked_source(&SourceGraph::from_owned([missing]), 0, source).is_err());

        let mut renamed = input();
        renamed.path = "other.css".into();
        assert!(checked_source(&SourceGraph::from_owned([renamed]), 0, source).is_err());

        let mut mismatched = input();
        mismatched.facts.commonjs = !mismatched.facts.commonjs;
        let error = checked_source(&SourceGraph::from_owned([mismatched]), 0, source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("parser configuration mismatch"), "{error}");

        for field in ["commonjs", "typescript", "jsx", "definition"] {
            let mut forged = input();
            let mut value = serde_json::to_value(forged.provenance.as_ref().unwrap()).unwrap();
            let original = value["config"][field].as_bool().unwrap();
            value["config"][field] = serde_json::Value::Bool(!original);
            forged.provenance = Some(serde_json::from_value(value).unwrap());
            let error = checked_source(&SourceGraph::from_owned([forged]), 0, source)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("parser configuration mismatch"),
                "{field}: {error}"
            );
        }

        let mut forged = input();
        forged.facts.commonjs = !forged.facts.commonjs;
        let mut value = serde_json::to_value(forged.provenance.as_ref().unwrap()).unwrap();
        value["config"]["commonjs"] = serde_json::Value::Bool(forged.facts.commonjs);
        forged.provenance = Some(serde_json::from_value(value).unwrap());
        let error = checked_source(&SourceGraph::from_owned([forged]), 0, source).unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("prepared resource identity/parser configuration mismatch"),
            "{error:#}"
        );

        let mut changed = input();
        changed.facts.emission.segments[0].connector_only =
            !changed.facts.emission.segments[0].connector_only;
        assert!(checked_source(&SourceGraph::from_owned([changed]), 0, source).is_err());
    }
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
fn split_declarations_materialize_independently_with_lexical_semantics() {
    for source in [
        "export const a = 1, b = a + 1; console.log(a, b);",
        "export let a, b = 2; a = b; console.log(a, b);",
        "var a = 1, a = a + 1; console.log(a);",
        "const {a = 1} = {}, [b = a] = []; console.log(a, b);",
        "const a = b, b = 1; console.log(a);",
        "const a = (() => b)(), b = 1; console.log(a);",
    ] {
        compare(source, "import './input.mjs';", true);
    }
    let output = compare(
        "const unusedFirst = 1, live = 2, unusedLast = 3; console.log(live);",
        "import './input.mjs';",
        false,
    );
    assert!(!output.contains("unusedFirst"), "{output}");
    assert!(!output.contains("unusedLast"), "{output}");
    assert!(output.contains("const live = 2;"), "{output}");
}

#[test]
fn tree_shakes_real_code_and_preserves_shadowing_hoisting_and_order() {
    let source = r#"
#!/not-a-real-hashbang
"#;
    // A hashbang is only valid at byte zero.
    assert!(ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("bad.mjs"),
        ast_segmenter::ModuleFormat::Esm,
    )
    .is_err());
    let source = r#"#!/usr/bin/env node
// copyright é
'use strict';
function unused() { throw new Error("must disappear"); }
console.log(hoisted(), before);
var before = 3;
function hoisted() { return 7; }
let x = 1;
{ let x = 2; console.log(x); }
console.log(x, /*# sourceMappingURL=obsolete.map */before);
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
fn optional_pure_keeps_await_in_its_original_lexical_context() {
    for invocation in [
        "globalThis.run?.(await Promise.resolve(1))",
        "globalThis?.[await Promise.resolve('run')](1)",
    ] {
        let source = format!("globalThis.run = () => {{}}; /*#__PURE__*/ {invocation};");
        let output = compare(&source, "import './input.mjs';", false);
        assert!(output.contains(invocation), "{output}");
    }
}

#[test]
fn optional_pure_still_optimizes_when_await_does_not_cross_a_guard() {
    for invocation in [
        "(await Promise.resolve(globalThis))?.run(1)",
        "(await Promise.resolve(globalThis)).run?.(1)",
        "globalThis.run?.(async () => { await Promise.resolve(1); })",
        "(globalThis?.run)(await Promise.resolve(1))",
    ] {
        let source = format!("globalThis.run = () => {{}}; /*#__PURE__*/ {invocation};");
        let output = compare(&source, "import './input.mjs';", false);
        assert!(!output.contains(invocation), "{output}");
        assert!(output.contains("void 0"), "{output}");
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
