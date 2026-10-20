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

#[test]
fn inferred_identity_and_parameter_hazards() {
    let analysis = source(
        "",
        r#"
        function identity(x) { const alias = x; return alias; }
        function read(x) { return x.value; }
        const object = {};
        const alias = identity(object);
        read(alias);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 5]);
    assert_eq!(analysis.roots[2].value, analysis.roots[3].value);
}

#[test]
fn inferred_functions_are_deferred_and_support_arrows() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function unused(x) { x.value = unknown(); }
        const unusedArrow = () => unknown();
        const pure = x => x + 1;
        pure(1);
        const log = function(x) { console.log(x); };
        log(1);
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, false, false, false, false, true]
    );
}

#[test]
fn inferred_closures_capture_each_invocation_separately() {
    let analysis = source(
        "",
        r#"
        function capture(x) { return () => x; }
        const first = {};
        const second = {};
        const readFirst = capture(first);
        const readSecond = capture(second);
        const a = readFirst();
        const b = readSecond();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 7]);
    assert_eq!(analysis.roots[1].value, analysis.roots[5].value);
    assert_eq!(analysis.roots[2].value, analysis.roots[6].value);
    assert_ne!(analysis.roots[5].value, analysis.roots[6].value);
}

#[test]
fn inferred_operations_track_symbols_to_hazard_markers() {
    for (body, hazard) in [
        ("return x.value", Hazard::PropertyRead),
        ("const alias = x; return alias.value", Hazard::PropertyRead),
        ("return +x", Hazard::ToPrimitive),
        ("return `value: ${x}`", Hazard::ToPrimitive),
        ("return 'field' in x", Hazard::In),
        ("return 1 instanceof x", Hazard::Instanceof),
    ] {
        let analysis = source("", &format!("function check(x) {{ {body}; }} check(1);"));
        assert_eq!(effects(&analysis), vec![false, false], "{body}");
        let mut analysis = source(
            "",
            &format!("function check(x) {{ {body}; }} const value = 1; check(value);"),
        );
        let value = analysis.roots[1].value;
        analysis
            .context
            .update(value, Hazard::Construct, analysis.context.effectful());
        assert_eq!(
            effects(&analysis),
            vec![false; 3],
            "{body}: unrelated hazard"
        );
        analysis
            .context
            .update(value, hazard, analysis.context.effectful());
        assert_eq!(effects(&analysis), vec![false, false, true], "{body}");
    }
}

#[test]
fn inferred_wrappers_preserve_updates_and_return_aliases() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function install(target) {
            const alias = target;
            return Object.defineProperty(alias, "value", {});
        }
        const object = {};
        object.value;
        const alias = install(object);
        alias.value;
    "#,
    );
    assert_eq!(effects(&analysis), vec![false, false, true, true, true]);
    assert_eq!(analysis.roots[1].value, analysis.roots[3].value);
    let uncalled = source(
        DECLARATIONS,
        r#"
        function install(target) { Object.defineProperty(target, "value", {}); }
        const object = {};
        object.value;
    "#,
    );
    assert_eq!(effects(&uncalled), vec![false; 3]);
}

#[test]
fn inferred_calls_resolve_imports_callbacks_and_lexical_shadows() {
    let analysis = source(
        DECLARATIONS,
        r#"
        import {join as combine} from "node:path";
        function join(a, b) { return combine(a, b); }
        function invoke(callback, x) { return callback(x); }
        function read(x) { return x.value; }
        function outer() {
            function read(x) { return x + 1; }
            return read(1);
        }
        join("a", "b");
        invoke(read, {});
        outer();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 7]);
    let effectful = source(
        DECLARATIONS,
        r#"
        function invoke(callback, x) { return callback(x); }
        invoke(console.log, 1);
    "#,
    );
    assert_eq!(effects(&effectful), vec![false, true]);
}

#[test]
fn inferred_captures_can_be_initialized_after_function_definitions() {
    let analysis = source(
        "",
        r#"
        function read() { return object.value; }
        function forward(x) { return identity(x); }
        function identity(x) { return x; }
        const object = {};
        const alias = forward(object);
        read();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 6]);
    assert_eq!(analysis.roots[3].value, analysis.roots[4].value);
}

#[test]
fn inferred_fresh_values_and_nested_declarations_are_per_call() {
    let analysis = source(
        "",
        r#"
        function fresh() {
            function identity(x) { return x; }
            return identity({});
        }
        const first = fresh();
        const second = fresh();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_ne!(analysis.roots[1].value, analysis.roots[2].value);
}

#[test]
fn inferred_returns_join_branches_and_include_fallthrough() {
    let mut analysis = source(
        "",
        r#"
        function choose(flag, left, right) {
            if (flag) { return left; } else { return right; }
            unknown();
        }
        const left = {};
        const right = {};
        const result = choose(true, left, right);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 4]);
    let result = analysis.roots[3].value;
    analysis
        .context
        .update(result, Hazard::PropertyRead, analysis.context.effectful());
    let evaluation = analysis.context.evaluate();
    assert!(evaluation
        .hazards(analysis.roots[1].value)
        .get(Hazard::PropertyRead));
    assert!(evaluation
        .hazards(analysis.roots[2].value)
        .get(Hazard::PropertyRead));
    let analysis = source(
        "",
        r#"
        function maybe(flag, x) { if (flag) return x; }
        maybe(false, {});
        function nothing() {}
        const result = nothing();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 4]);
    assert_eq!(analysis.roots[3].value, analysis.context.undefined());
}

#[test]
fn inferred_recursion_shares_exact_contexts_and_bounds_changing_ones() {
    for javascript in [
        "function recur(x) { recur(x); return x; } recur(1);",
        "function a(x) { b(x); return x; } function b(x) { a(x); } a(1);",
        "const recur = function self(x) { self(x); return x; }; recur(1);",
    ] {
        let analysis = source("", javascript);
        assert!(
            effects(&analysis).iter().all(|effect| !effect),
            "{javascript}"
        );
    }
    let analysis = source("", "function grow(x) { grow({}); } grow(1);");
    assert_eq!(effects(&analysis), vec![false, true]);
}

#[test]
fn inferred_callbacks_work_in_declarations_without_executing_latent_updates() {
    let declarations = r#"
        fn install(target, callback) {
            call_effects = false
            updates target.property_read_effects |= call(callback, this = target)
            returns = target
        }
        fn mutate(target) {
            call_effects = true
            updates target.property_read_effects |= true
            returns void
        }
        fn invoke(callback, target) { call_effects = call(callback, target) returns void }
    "#;
    let analysis = source(
        declarations,
        r#"
        function callback() { return this.value; }
        function install() {}
        const object = {};
        object.value;
        install(object, callback);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 5]);
    let analysis = source(
        declarations,
        r#"
        function mutate(target) {}
        function invoke(callback, target) {}
        function callback(target) { mutate(target); }
        const object = {};
        object.value;
        invoke(callback, object);
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, false, false, false, true, true]
    );
}

#[test]
fn inferred_unsupported_bodies_remain_conservative_only_when_called() {
    for function in [
        "function f(x = unknown()) {}",
        "function f({x}) {}",
        "function f(...args) {}",
        "async function f() {}",
        "function* f() {}",
        "function f() { while (true) unknown(); }",
        "function f() { captured = 1; }",
        "function f() { return arguments[0]; }",
    ] {
        let uncalled = source("", function);
        assert_eq!(effects(&uncalled), vec![false], "{function}");
        let called = source("", &format!("{function} f();"));
        assert_eq!(effects(&called), vec![false, true], "{function}");
    }
}

#[test]
fn inferred_callable_hazards_do_not_replace_specialized_call_effects() {
    let declarations = r#"
        fn probe(callback) { call_effects = callback.call_effects returns void }
        global fn log(...) { call_effects = true returns void }
    "#;
    let pure = source(
        declarations,
        r#"
        function probe(callback) {}
        const callback = () => 1;
        probe(callback);
        callback();
    "#,
    );
    assert_eq!(effects(&pure), vec![false; 4]);
    let effectful = source(
        declarations,
        r#"
        function probe(callback) {}
        const callback = () => log();
        probe(callback);
    "#,
    );
    assert_eq!(effects(&effectful), vec![false, false, true]);
    let mut analysis = source(
        "",
        r#"
        const read = x => x.value;
        const object = {};
        read(object);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    let function = analysis.roots[0].value;
    analysis
        .context
        .update(function, Hazard::Call, analysis.context.effectful());
    assert_eq!(effects(&analysis), vec![false, false, true]);
}

#[test]
fn inferred_latent_callbacks_do_not_execute_updates_or_unknown_code() {
    let analysis = source(
        r#"
        fn install(target, callback) {
            call_effects = false
            updates target.property_read_effects |= call(callback)
            returns void
        }
    "#,
        r#"
        function install(target, callback) {}
        const victim = {};
        victim.value;
        const callback = () => unknown();
        const target = {};
        install(target, callback);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 6]);
    let evaluation = analysis.context.evaluate();
    assert!(evaluation
        .hazards(analysis.roots[4].value)
        .get(Hazard::PropertyRead));
    assert!(!evaluation
        .hazards(analysis.roots[1].value)
        .get(Hazard::PropertyRead));
    let analysis = source(
        r#"
        fn install(target, callback, input) {
            call_effects = false
            updates target.property_read_effects |= call(callback, input)
            returns void
        }
        global fn mutate(target) {
            call_effects = false
            updates target.property_read_effects |= true
            returns void
        }
    "#,
        r#"
        function install(target, callback, input) {}
        function callback(input) { mutate(input); return input.value; }
        const input = {};
        input.value;
        const target = {};
        install(target, callback, input);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 6]);
    let evaluation = analysis.context.evaluate();
    assert!(evaluation
        .hazards(analysis.roots[4].value)
        .get(Hazard::PropertyRead));
    assert!(!evaluation
        .hazards(analysis.roots[2].value)
        .get(Hazard::PropertyRead));
}

#[test]
fn inferred_arrows_capture_lexical_this_and_ordinary_functions_use_receiver() {
    let mut analysis = source("", "const make = function() { return () => this; };");
    let receiver = analysis.context.value(Hazards::NONE);
    let make = analysis.roots[0].value;
    let closure = analysis.context.call(make, &[], Some(receiver));
    let other = analysis.context.value(Hazards::NONE);
    let result = analysis.context.call(closure.value, &[], Some(other));
    assert_eq!(result.value, receiver);
    let evaluation = analysis.context.evaluate();
    assert!(!evaluation.effect(closure.effects));
    assert!(!evaluation.effect(result.effects));
}

#[test]
fn inferred_parameters_allow_missing_and_extra_arguments_and_local_rebinding() {
    let analysis = source(
        "",
        r#"
        function identity(x) { return x; }
        const missing = identity();
        const value = {};
        const extra = identity(value, 1, 2);
        function rebind(x) { var x = 1; return x; }
        const rebound = rebind(value);
        const unchanged = identity(value);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 7]);
    assert_eq!(analysis.roots[1].value, analysis.context.undefined());
    assert_eq!(analysis.roots[2].value, analysis.roots[3].value);
    assert_eq!(analysis.roots[2].value, analysis.roots[6].value);
    assert_ne!(analysis.roots[2].value, analysis.roots[5].value);
    let analysis = source(
        "",
        r#"
        function identity(x) { return x; }
        var value = {};
        const original = value;
        var value = {};
        const alias = identity(original);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 5]);
    assert_eq!(analysis.roots[1].value, analysis.roots[4].value);
    assert_ne!(analysis.roots[3].value, analysis.roots[4].value);
}

#[test]
fn inferred_graphs_propagate_late_capture_hazards_across_worker_counts() {
    let mut analysis = source(
        "",
        r#"
        function make(input) { return () => input.value; }
        const input = {};
        const read = make(input);
        read();
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 4]);
    let input = analysis.roots[1].value;
    analysis
        .context
        .update(input, Hazard::PropertyRead, analysis.context.effectful());
    for workers in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let evaluation = pool.install(|| analysis.context.evaluate());
        let effects: Vec<_> = analysis
            .roots
            .iter()
            .map(|root| evaluation.effect(root.effects))
            .collect();
        assert_eq!(effects, vec![false, false, true, true]);
    }
}
