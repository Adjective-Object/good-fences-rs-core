use std::path::Path;

use crate::*;

const DECLARATIONS: &str = r#"
global fn console.log(...) {
  call_effects = true
  returns void
}
global fn Number(value) {
  call_effects = value.toprimitive_effects
  returns.*_effects = false
}
global fn Object.defineProperty(target, key, descriptor) {
  call_effects = true
  updates target.*_effects |= true
  returns = target
}
module "node:path" {
  export fn join(...paths) {
    call_effects = false
    returns.*_effects = false
  }
}
module "node:buffer" {
  export fn Buffer.alloc(size) {
    call_effects = false
    returns.property_read_effects = false
  }
}
fn installGetter(target, getter) {
  call_effects = true
  updates target.property_read_effects |= call(getter, this = target)
  returns = target
}
"#;

fn context(declarations: &str) -> Context {
    Context::from_local_facts(parse_declarations(declarations).unwrap()).unwrap()
}

fn local(context: &Context, name: &str) -> ValueId {
    context.definition(&Binding::Local(name.into())).unwrap()
}

fn source(declarations: &str, javascript: &str) -> SourceAnalysis {
    analyze_source(
        javascript,
        Path::new("test.js"),
        parse_declarations(declarations).unwrap(),
    )
    .unwrap()
}

fn effects(analysis: &SourceAnalysis) -> Vec<bool> {
    let evaluation = analysis.context.evaluate();
    analysis
        .roots
        .iter()
        .map(|root| evaluation.effect(root.effects))
        .collect()
}

#[test]
fn parses_node_contracts_and_document_examples() {
    let facts = parse_declarations(DECLARATIONS).unwrap();
    assert_eq!(facts.definitions.len(), 6);
    assert_eq!(facts.definitions[0].returns, Returns::Void);
    assert!(facts.definitions[0].variadic);
    assert_eq!(
        facts.definitions[4].binding,
        Binding::Module {
            module: "node:buffer".into(),
            path: vec!["Buffer".into(), "alloc".into()]
        }
    );
    let document = include_str!("../../../NOTES.effect-analysis.md");
    for block in document.split("```text\n").skip(1) {
        parse_declarations(block.split("```").next().unwrap()).unwrap();
    }
}

#[test]
fn parser_reports_bad_syntax_and_references() {
    for text in [
        "fn broken(",
        r#"module "unterminated"#,
        "global fn f() { call_effects = maybe }",
        "fn f(x) { call_effects = y.toprimitive_effects }",
        "fn f(x) { updates y.call_effects |= true }",
        "fn f(x) { returns = y }",
        "fn f() { returns void returns.call_effects = false }",
        "fn f(x) { call_effects = x.no_such_effects }",
        "fn f() {} fn f() {}",
        "fn f(x, x) {}",
        "fn f(x, ...x) {}",
        "fn f(..., x) {}",
        "fn f() { call_effects = false call_effects = true }",
        "fn f(x) { updates x.call_effects = true }",
        "fn f(x) { call_effects = call(x, ...missing) }",
        "fn f(x) { returns = x | x }",
        "fn f() { returns void returns void }",
    ] {
        assert!(parse_declarations(text).is_err(), "accepted {text}");
    }
    let error = parse_declarations("fn f() { @ }").unwrap_err().to_string();
    assert!(error.contains("byte 9"), "{error}");
}

#[test]
fn callable_parameter_names_and_owned_fact_validation() {
    let facts = parse_declarations("fn check(call) { call_effects = call.call_effects }").unwrap();
    let mut invalid = facts.clone();
    invalid.definitions[0].parameters.clear();
    assert!(Context::from_local_facts(invalid).is_err());
    let mut context = Context::from_local_facts(facts).unwrap();
    let argument = context.value(Hazards::NONE);
    let result = context.call(local(&context, "check"), &[argument], None);
    assert!(!context.evaluate().effect(result.effects));
}

#[test]
fn wildcard_defaults_and_omitted_hazards_are_distinct() {
    let mut context = context(
        r#"
        fn ordinary() { call_effects = false }
        fn defaults() {
          returns.property_read_effects = false
          returns.*_effects = true
          call_effects = false
          *_effects = true
        }
    "#,
    );
    let ordinary = context.call(local(&context, "ordinary"), &[], None);
    let defaults = context.call(local(&context, "defaults"), &[], None);
    let evaluation = context.evaluate();
    assert!(!evaluation.effect(ordinary.effects));
    assert_eq!(evaluation.hazards(ordinary.value), Hazards::UNKNOWN);
    assert!(!evaluation.effect(defaults.effects));
    assert!(!evaluation.hazards(defaults.value).get(Hazard::PropertyRead));
    assert!(evaluation.hazards(defaults.value).get(Hazard::ToPrimitive));
}

#[test]
fn conditional_calls_and_missing_parameters() {
    let mut context =
        context("fn convert(x) { call_effects = x.toprimitive_effects returns.*_effects = false }");
    let function = local(&context, "convert");
    let primitive = context.value(Hazards::NONE);
    let safe = context.call(function, &[primitive], None);
    let missing = context.call(function, &[], None);
    let evaluation = context.evaluate();
    assert!(!evaluation.effect(safe.effects));
    assert!(!evaluation.effect(missing.effects));
    assert_eq!(evaluation.hazards(safe.value), Hazards::NONE);
    let hazardous = context.call(function, &[context.unknown()], None);
    let evaluation = context.evaluate();
    assert!(evaluation.effect(hazardous.effects));
    assert!(
        evaluation.effect(safe.effects),
        "unknown coercion may replace other call targets"
    );
}

#[test]
fn updates_revisit_earlier_safe_reads_and_passthrough_aliases() {
    let mut context = context(
        r#"
        fn identity(x) { call_effects = false returns = x }
        fn mutate(x) {
          call_effects = true
          updates x.property_read_effects |= true
          returns = x
        }
    "#,
    );
    let object = context.value(Hazards::NONE);
    let alias = context.call(local(&context, "identity"), &[object], None);
    assert_eq!(alias.value, object);
    let read = context.hazard(alias.value, Hazard::PropertyRead);
    let dependent = context.any([read, alias.effects]);
    assert!(!context.evaluate().effect(dependent));
    context.call(local(&context, "mutate"), &[object], None);
    let evaluation = context.evaluate();
    assert!(evaluation.effect(dependent));
    assert!(!evaluation.hazards(object).get(Hazard::ToPrimitive));
}

#[test]
fn fresh_returns_do_not_alias_other_calls() {
    let mut context = context("fn fresh() { call_effects = false returns.*_effects = false }");
    let first = context.call(local(&context, "fresh"), &[], None);
    let second = context.call(local(&context, "fresh"), &[], None);
    assert_ne!(first.value, second.value);
    context.update(first.value, Hazard::PropertyRead, context.effectful());
    let evaluation = context.evaluate();
    assert!(evaluation.hazards(first.value).get(Hazard::PropertyRead));
    assert!(!evaluation.hazards(second.value).get(Hazard::PropertyRead));
}

#[test]
fn conditional_getter_effects_are_latent_and_use_receiver() {
    let mut context = context(
        r#"
        fn getter() { call_effects = this.toprimitive_effects returns void }
        fn install(target, getter) {
          call_effects = false
          updates target.property_read_effects |= call(getter, this = target)
          returns = target
        }
    "#,
    );
    let getter = local(&context, "getter");
    let object = context.value(Hazards::NONE);
    let result = context.call(local(&context, "install"), &[object, getter], None);
    let read = context.hazard(object, Hazard::PropertyRead);
    assert!(!context.evaluate().effect(read));
    context.update(object, Hazard::ToPrimitive, context.effectful());
    let evaluation = context.evaluate();
    assert!(evaluation.effect(read));
    assert!(
        !evaluation.effect(result.effects),
        "installing a getter must not execute it"
    );
    assert_eq!(object, result.value);
}

#[test]
fn rest_parameters_and_forwarded_calls() {
    let mut context = context(
        r#"
        fn convert(x) { call_effects = x.toprimitive_effects }
        fn forward(f, ...args) { call_effects = call(f, ...args) }
        fn explicit(f, x) { call_effects = call(f, x) | false }
    "#,
    );
    let convert = local(&context, "convert");
    let primitive = context.value(Hazards::NONE);
    let safe = context.call(local(&context, "forward"), &[convert, primitive], None);
    assert!(!context.evaluate().effect(safe.effects));
    let unsafe_call = context.call(
        local(&context, "explicit"),
        &[convert, context.unknown()],
        None,
    );
    let evaluation = context.evaluate();
    assert!(evaluation.effect(unsafe_call.effects));
}

#[test]
fn recursive_summaries_converge_and_propagate_new_hazards() {
    let mut context = context("fn recursive(f) { call_effects = call(f, f) }");
    let function = local(&context, "recursive");
    let result = context.call(function, &[function], None);
    assert!(!context.evaluate().effect(result.effects));
    context.update(function, Hazard::Call, context.effectful());
    assert!(context.evaluate().effect(result.effects));
}

#[test]
fn parallel_fixed_points_match_across_worker_counts() {
    let mut context = context("");
    let values: Vec<_> = (0..128).map(|_| context.value(Hazards::NONE)).collect();
    for (index, value) in values.iter().enumerate() {
        let read = context.hazard(*value, Hazard::PropertyRead);
        context.update(
            values[(index + 1) % values.len()],
            Hazard::PropertyRead,
            read,
        );
        context.update(
            values[(index + 7) % values.len()],
            Hazard::PropertyRead,
            read,
        );
    }
    let untouched = context.value(Hazards::NONE);
    context.update(values[0], Hazard::PropertyRead, context.effectful());
    for workers in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let evaluation = pool.install(|| context.evaluate());
        assert!(values
            .iter()
            .all(|value| evaluation.hazards(*value).get(Hazard::PropertyRead)));
        assert_eq!(evaluation.hazards(untouched), Hazards::NONE);
        assert!(values
            .iter()
            .all(|value| !evaluation.hazards(*value).get(Hazard::ToPrimitive)));
    }
}

#[test]
fn unknown_execution_hazards_existing_and_future_values() {
    let mut context = context("");
    let before = context.value(Hazards::NONE);
    context.call(context.unknown(), &[], None);
    let after = context.value(Hazards::NONE);
    let evaluation = context.evaluate();
    assert_eq!(evaluation.hazards(before), Hazards::UNKNOWN);
    assert_eq!(evaluation.hazards(after), Hazards::UNKNOWN);
}

#[test]
fn source_matches_globals_import_aliases_and_namespace_members() {
    let analysis = source(
        DECLARATIONS,
        r#"
        import { join as combine } from "node:path";
        import * as buffers from "node:buffer";
        const a = 1;
        const b = a + 1;
        const c = combine("a", "b");
        buffers.Buffer.alloc(8);
        console["log"](c);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false, false, false, false, true]);
    let matches: Vec<_> = analysis
        .matches
        .iter()
        .map(|matched| format!("{:?}", matched.binding))
        .collect();
    insta::assert_debug_snapshot!(matches, @r#"
    [
        "Module { module: \"node:path\", path: [\"join\"] }",
        "Module { module: \"node:buffer\", path: [\"Buffer\", \"alloc\"] }",
        "Global([\"console\", \"log\"])",
    ]
    "#);
}

#[test]
fn lexical_shadows_and_wrong_modules_do_not_match() {
    for javascript in [
        "const console = {log: () => {}}; console.log(1);",
        "function Number(x) { unknown(x) } Number(1);",
        r#"import { join } from "other:path"; join("a", "b");"#,
        "const Number = unknown; Number(1);",
    ] {
        let analysis = source(DECLARATIONS, javascript);
        assert_eq!(effects(&analysis).last(), Some(&true), "{javascript}");
        assert!(analysis.matches.is_empty(), "{javascript}");
    }
}

#[test]
fn source_updates_invalidate_earlier_reads_through_return_aliases() {
    let analysis = source(
        DECLARATIONS,
        r#"
        const a = {};
        const b = a;
        const before = b.hello;
        const alias = Object.defineProperty(a, "hello", {});
        const after = alias.hello;
    "#,
    );
    assert_eq!(effects(&analysis), vec![false, false, true, true, true]);
    assert_eq!(analysis.roots[0].value, analysis.roots[3].value);
}

#[test]
fn source_local_summaries_apply_only_when_called() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function installGetter(target, getter) {}
        const a = {};
        const before = a.hello;
        installGetter(a, console.log);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false, false, true, true]);
    let no_call = source(
        DECLARATIONS,
        r#"
        function installGetter(target, getter) {}
        const a = {};
        const before = a.hello;
    "#,
    );
    assert_eq!(effects(&no_call), vec![false, false, false]);
}

#[test]
fn source_coercion_and_ordinary_reads_depend_on_hazards() {
    let analysis = source(
        DECLARATIONS,
        r#"
        const a = 1;
        Number(a);
        const b = {};
        b.field;
        const fn = () => console.log("later");
    "#,
    );
    assert_eq!(effects(&analysis), vec![false, false, false, false, false]);
}

#[test]
fn executed_getters_and_coercion_widen_reachable_mutations() {
    for javascript in [
        "const victim = {}; victim.x; const obj = {get x() { unknown() }}; obj.x;",
        "const victim = {}; victim.x; const obj = {valueOf() { unknown() }}; +obj;",
        "const victim = {}; victim.x; const obj = {valueOf() { unknown() }}; Number(obj);",
    ] {
        let analysis = source(DECLARATIONS, javascript);
        assert_eq!(effects(&analysis), vec![false, true, false, true]);
    }
}

#[test]
fn recursive_argument_growth_is_bounded() {
    let mut context =
        context("fn grow(f, ...args) { call_effects = call(f, f, ...args, ...args) }");
    let function = local(&context, "grow");
    let argument = context.value(Hazards::NONE);
    let result = context.call(function, &[function, argument], None);
    assert!(context.evaluate().effect(result.effects));
}

#[test]
fn unknown_callback_summaries_widen_mutations() {
    let mut context = context("fn invoke(callback) { call_effects = call(callback) }");
    let victim = context.value(Hazards::NONE);
    let result = context.call(local(&context, "invoke"), &[context.unknown()], None);
    let evaluation = context.evaluate();
    assert!(evaluation.effect(result.effects));
    assert_eq!(evaluation.hazards(victim), Hazards::UNKNOWN);
}

#[test]
fn exported_classes_and_disposal_are_not_skipped() {
    for javascript in [
        "export class C extends unknown() {}",
        "export class C { static { unknown() } }",
        "using resource = value;",
        "await using resource = value;",
    ] {
        let analysis = source(DECLARATIONS, javascript);
        assert_eq!(effects(&analysis), vec![true], "{javascript}");
    }
    let ordinary_export = source(DECLARATIONS, "export const x = 1; export function f() {}");
    assert_eq!(effects(&ordinary_export), vec![false, false]);
}

#[test]
fn source_arguments_are_evaluated_even_for_pure_callees() {
    let analysis = source(
        DECLARATIONS,
        r#"
        import {join} from "node:path";
        join(console.log("argument"));
    "#,
    );
    assert_eq!(effects(&analysis), vec![true]);
}

#[test]
fn source_replacement_and_unknown_code_invalidate_trusted_calls() {
    for javascript in [
        "Number(1); Number = replacement;",
        "Number(1); console.log = replacement;",
        "Number(1); unknown();",
        "Number(1); if (condition) unknown();",
        "Number(1); delete console.log;",
    ] {
        let analysis = source(DECLARATIONS, javascript);
        assert!(
            effects(&analysis).iter().all(|effect| *effect),
            "{javascript}"
        );
    }
}

#[test]
fn source_conditional_aliases_receive_updates() {
    let analysis = source(
        DECLARATIONS,
        r#"
        const a = {};
        const b = {};
        const before = a.x;
        const choice = condition ? a : b;
        Object.defineProperty(choice, "x", {});
        const after = b.x;
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, false, true, false, true, true]
    );
}

#[test]
fn source_prototype_getters_and_unknown_update_targets_are_conservative() {
    let prototype = source(
        DECLARATIONS,
        r#"
        const prototype = {get x() { console.log("getter") }};
        const object = {__proto__: prototype};
        object.x;
    "#,
    );
    assert_eq!(effects(&prototype), vec![false, false, true]);
    let nested = source(
        DECLARATIONS,
        r#"
        const inner = {};
        const outer = {inner};
        inner.x;
        Object.defineProperty(outer.inner, "x", {});
    "#,
    );
    assert_eq!(effects(&nested), vec![false, false, true, true]);
}

#[test]
fn source_arities_spreads_and_construction_fall_back_conservatively() {
    for javascript in ["Number(1, 2);", "Number(...args);", "new Number(1);"] {
        let analysis = source(DECLARATIONS, javascript);
        assert_eq!(effects(&analysis), vec![true], "{javascript}");
    }
}

#[test]
fn source_errors_are_not_recovered_into_purity() {
    for javascript in ["const broken = ;", "const x = 1; const x = 2;"] {
        assert!(analyze_source(javascript, Path::new("broken.js"), LocalFacts::default()).is_err());
    }
}

#[test]
fn source_ast_entrypoint_does_not_need_reparsing() {
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_utils_parse::parse_ts(&allocator, "const a: number = 1; Number(a);");
    assert!(parsed.errors.is_empty());
    let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
    assert!(semantic.errors.is_empty());
    let analysis = extract_from_ast(
        &parsed.program,
        &semantic.semantic,
        parse_declarations(DECLARATIONS).unwrap(),
    )
    .unwrap();
    assert_eq!(effects(&analysis), vec![false, false]);
}
