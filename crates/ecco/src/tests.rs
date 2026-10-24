use std::path::Path;
use std::sync::Arc;

use crate::views::{BindingView, ExternalReferenceView};
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

#[test]
fn unknown_contract_behavior_is_not_executed_while_building_facts() {
    let mut context = context("fn opaque() { call_effects = unknown() returns unknown }");
    let value = context.value(Hazards::NONE);
    assert_eq!(context.evaluate().hazards(value), Hazards::NONE);
    let result = context.call(local(&context, "opaque"), &[], None);
    assert_eq!(result.value(), context.unknown());
    assert!(context.evaluate().effect(result.effects()));
    assert_eq!(context.evaluate().hazards(value), Hazards::UNKNOWN);
    parse_declarations("fn f(unknown) { call_effects = unknown.call_effects returns = unknown }")
        .unwrap();
}

#[test]
fn argument_dependent_contracts_specialize_without_losing_bare_callable_hazards() {
    let mut context = context(
        r#"
        fn read(value) { call_effects = value.property_read_effects returns void }
        fn invoke(callback, value) { call_effects = call(callback, value) returns void }
    "#,
    );
    let read = local(&context, "read");
    let value = context.value(Hazards::NONE);
    let result = context.call(local(&context, "invoke"), &[read, value], None);
    assert!(context.evaluate().hazards(read).get(Hazard::Call));
    assert!(!context.evaluate().effect(result.effects()));
    context.update(read, Hazard::Call, context.effectful());
    assert!(context.evaluate().effect(result.effects()));
}

fn effects(analysis: &SourceAnalysis) -> Vec<bool> {
    let evaluation = analysis.context.evaluate();
    analysis
        .roots
        .iter()
        .map(|root| evaluation.effect(root.effects))
        .collect()
}

fn parsed(javascript: &str) -> Arc<ParsedFile> {
    Arc::new(parse_source(javascript, Path::new("test.js"), LocalFacts::default()).unwrap())
}

fn symbol(file: &ParsedFile, name: &str) -> SymbolId {
    file.symbols()
        .iter()
        .find(|symbol| symbol.name() == name)
        .unwrap()
        .id()
}

#[test]
fn owned_precursors_roundtrip_without_executing_or_saving_analysis() {
    let parsed = parse_source(
        "function make(x) { return () => x.value; } const input = {}; \
         const read = make(input); /*@__PURE__*/ opaque(); read();",
        Path::new("owned.js"),
        LocalFacts::default(),
    )
    .unwrap();
    let bytes = serde_cbor::to_vec(&parsed).unwrap();
    let parsed: ParsedFile = serde_cbor::from_slice(&bytes).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.functions().len(), 2);
    assert_eq!(parsed.roots().len(), 5);
    let parsed = Arc::new(parsed);
    for _ in 0..2 {
        let mut analysis = Analysis::new();
        analysis.add_file((), parsed.clone()).unwrap();
        let result = analysis
            .solve(&|_: &(), _: ExternalReferenceView<'_>| Ok(Resolution::Unknown))
            .unwrap();
        assert!(result.files[&()]
            .roots
            .iter()
            .all(|root| !result.evaluation.effect(root.effects)));
    }
    assert_eq!(bytes, serde_cbor::to_vec(&*parsed).unwrap());
}

#[test]
fn graph_calls_use_external_symbol_resolutions_and_actual_arguments() {
    let library = parsed("export function read(input) { return input.value; }");
    let read = symbol(&library, "read");
    let library: ParsedFile =
        serde_cbor::from_slice(&serde_cbor::to_vec(&*library).unwrap()).unwrap();
    let library = Arc::new(library);
    for (argument, effectful) in [("{}", false), ("{ get value() {} }", true)] {
        for reverse in [false, true] {
            let entry = parsed(&format!(
                "import {{ read as alias }} from 'resolved-by-caller'; alias({argument});"
            ));
            let mut analysis = Analysis::new();
            let files = if reverse {
                [("library", library.clone()), ("entry", entry)]
            } else {
                [("entry", entry), ("library", library.clone())]
            };
            for (key, file) in files {
                analysis.add_file(key, file).unwrap();
            }
            let result = analysis
                .solve(&|from: &&str, reference: ExternalReferenceView<'_>| {
                    assert_eq!(*from, "entry");
                    assert_eq!(
                        reference.binding().to_owned(),
                        Binding::Module {
                            module: "resolved-by-caller".into(),
                            path: vec!["read".into()],
                        }
                    );
                    Ok(Resolution::Symbol(Symbol {
                        file: "library",
                        local: read,
                    }))
                })
                .unwrap();
            assert_eq!(
                result
                    .evaluation
                    .effect(result.files["entry"].roots[0].effects),
                effectful,
            );
        }
    }
}

#[test]
fn graph_links_cyclic_functions_before_instantiating_calls() {
    let a = parsed("import { b } from 'b'; export function a(value) { return b(value); } a({});");
    let b = parsed(
        "import { a } from 'a'; export function b(value) { \
         if (value) return a(value); return value.key; }",
    );
    let a_symbol = symbol(&a, "a");
    let b_symbol = symbol(&b, "b");
    for reverse in [false, true] {
        let mut analysis = Analysis::new();
        let files = if reverse {
            [("b", b.clone()), ("a", a.clone())]
        } else {
            [("a", a.clone()), ("b", b.clone())]
        };
        for (key, file) in files {
            analysis.add_file(key, file).unwrap();
        }
        let result = analysis
            .solve(
                &|_: &&str, reference: ExternalReferenceView<'_>| match reference.binding() {
                    BindingView::Module { module: "a", .. } => Ok(Resolution::Symbol(Symbol {
                        file: "a",
                        local: a_symbol,
                    })),
                    BindingView::Module { module: "b", .. } => Ok(Resolution::Symbol(Symbol {
                        file: "b",
                        local: b_symbol,
                    })),
                    _ => eyre::bail!("unexpected reference"),
                },
            )
            .unwrap();
        assert!(result
            .files
            .values()
            .flat_map(|file| &file.roots)
            .all(|root| !result.evaluation.effect(root.effects)));
    }
}

#[test]
fn graph_materializes_foreign_initializers_and_anonymous_default_roots_lazily() {
    let library = parsed("export const input = {}; export default function () { return input; }");
    let input = symbol(&library, "input");
    let entry =
        parsed("import get, { input } from 'library'; const alias = get(); alias === input;");
    for reverse in [false, true] {
        let mut analysis = Analysis::new();
        let files = if reverse {
            [("library", library.clone()), ("entry", entry.clone())]
        } else {
            [("entry", entry.clone()), ("library", library.clone())]
        };
        for (key, file) in files {
            analysis.add_file(key, file).unwrap();
        }
        let result = analysis
            .solve(
                &|_: &&str, reference: ExternalReferenceView<'_>| match reference.binding() {
                    BindingView::Module { path, .. } if path.iter().eq(["default"]) => {
                        Ok(Resolution::Root {
                            file: "library",
                            root: RootId(1),
                        })
                    }
                    BindingView::Module { path, .. } if path.iter().eq(["input"]) => {
                        Ok(Resolution::Symbol(Symbol {
                            file: "library",
                            local: input,
                        }))
                    }
                    _ => eyre::bail!("unexpected reference"),
                },
            )
            .unwrap();
        assert_eq!(
            result.files["entry"].roots[0].value,
            result.files["library"].roots[0].value
        );
        assert!(result
            .files
            .values()
            .flat_map(|file| &file.roots)
            .all(|root| !result.evaluation.effect(root.effects)));
    }
}

#[test]
fn graph_resolver_failures_and_invalid_saved_ids_are_errors() {
    let file = parsed("import { missing } from 'missing'; missing();");
    let mut analysis = Analysis::new();
    analysis.add_file((), file.clone()).unwrap();
    assert!(analysis.add_file((), file.clone()).is_err());
    let error = analysis
        .clone()
        .solve(&|_: &(), _: ExternalReferenceView<'_>| eyre::bail!("ambiguous caller export"))
        .unwrap_err();
    assert!(error.to_string().contains("ambiguous caller export"));
    assert!(analysis
        .clone()
        .solve(&|_: &(), _: ExternalReferenceView<'_>| {
            Ok(Resolution::Symbol(Symbol {
                file: (),
                local: SymbolId(u32::MAX),
            }))
        })
        .is_err());
    assert!(analysis
        .solve(&|_: &(), _: ExternalReferenceView<'_>| {
            Ok(Resolution::Root {
                file: (),
                root: RootId(1000),
            })
        })
        .is_err());

    let mut json = serde_json::to_value(&*file).unwrap();
    json["roots"][0]["function"]["returns"] = serde_json::json!([usize::MAX]);
    let invalid: ParsedFile = serde_json::from_value(json).unwrap();
    assert!(Analysis::new().add_file((), Arc::new(invalid)).is_err());
}

#[test]
fn graph_namespace_aliases_resolve_static_members_with_shared_identity() {
    let library = parsed("export const read = value => value.key;");
    let read = symbol(&library, "read");
    let entry =
        parsed("import * as ns from 'library'; const alias = ns; alias.read({}); ns === alias;");
    let mut analysis = Analysis::new();
    analysis.add_file("entry", entry).unwrap();
    analysis.add_file("library", library).unwrap();
    let result = analysis
        .solve(
            &|_: &&str, reference: ExternalReferenceView<'_>| match reference.binding() {
                BindingView::Module { path, .. } if path.is_empty() => {
                    Ok(Resolution::Namespace("library"))
                }
                BindingView::Module { path, .. } if path.iter().eq(["read"]) => {
                    Ok(Resolution::Symbol(Symbol {
                        file: "library",
                        local: read,
                    }))
                }
                _ => eyre::bail!("unexpected reference"),
            },
        )
        .unwrap();
    assert!(result
        .files
        .values()
        .flat_map(|file| &file.roots)
        .all(|root| !result.evaluation.effect(root.effects)));
}

#[test]
fn graph_callback_queries_subscribe_to_foreign_live_bindings() {
    let entry = Arc::new(parse_source(
        "import { callback } from 'library'; function read() { return callback(); } query(read);",
        Path::new("entry.js"),
        parse_declarations(
            "global fn query(callback) { call_effects = call(callback) returns void }",
        ).unwrap(),
    ).unwrap());
    for rebind in [false, true] {
        let library = parsed(&format!(
            "export var callback = () => 1; {}",
            if rebind {
                "var callback = () => opaque();"
            } else {
                ""
            },
        ));
        let callback = symbol(&library, "callback");
        let mut analysis = Analysis::new();
        analysis.add_file("entry", entry.clone()).unwrap();
        analysis.add_file("library", library).unwrap();
        let result = analysis
            .solve(
                &|_: &&str, reference: ExternalReferenceView<'_>| match reference.binding() {
                    BindingView::Module { .. } => Ok(Resolution::Symbol(Symbol {
                        file: "library",
                        local: callback,
                    })),
                    BindingView::Global(path) if path.iter().eq(["query"]) => {
                        Ok(Resolution::Declaration(reference.binding().to_owned()))
                    }
                    _ => Ok(Resolution::Unknown),
                },
            )
            .unwrap();
        assert_eq!(
            result
                .evaluation
                .effect(result.files["entry"].roots[1].effects),
            rebind,
        );
    }
}

#[test]
fn graph_initializers_execute_updates_even_when_demanded_by_latent_queries() {
    let library = Arc::new(
        parse_source(
            "export const value = install({});",
            Path::new("library.js"),
            parse_declarations(
                "global fn install(value) { call_effects = false \
             updates value.property_read_effects |= true returns = value }",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let value = symbol(&library, "value");
    let entry = Arc::new(
        parse_source(
            "import { value } from 'library'; function read() { return value.key; } query(read);",
            Path::new("entry.js"),
            parse_declarations(
                "global fn query(callback) { call_effects = call(callback) returns void }",
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let mut analysis = Analysis::new();
    analysis.add_file("entry", entry).unwrap();
    analysis.add_file("library", library).unwrap();
    let result = analysis
        .solve(
            &|_: &&str, reference: ExternalReferenceView<'_>| match reference.binding() {
                BindingView::Module { .. } => Ok(Resolution::Symbol(Symbol {
                    file: "library",
                    local: value,
                })),
                _ => Ok(Resolution::Declaration(reference.binding().to_owned())),
            },
        )
        .unwrap();
    assert!(result
        .evaluation
        .effect(result.files["entry"].roots[1].effects));
}

#[test]
fn graph_named_default_roots_preserve_the_hoisted_function_identity() {
    let library = parsed("export default function self() { return self(); }");
    let self_symbol = symbol(&library, "self");
    let entry = parsed("import run from 'library'; run();");
    let mut analysis = Analysis::new();
    analysis.add_file("entry", entry).unwrap();
    analysis.add_file("library", library).unwrap();
    let result = analysis
        .solve(&|_: &&str, _: ExternalReferenceView<'_>| {
            Ok(Resolution::Root {
                file: "library",
                root: RootId(0),
            })
        })
        .unwrap();
    assert_eq!(
        result.files["library"].roots[0].value,
        result.files["library"].symbols[&self_symbol],
    );
    assert!(!result
        .evaluation
        .effect(result.files["entry"].roots[0].effects));
}

#[test]
fn graph_local_contracts_are_scoped_to_their_parsed_files() {
    let mut analysis = Analysis::new();
    for (key, call_effects) in [("pure", false), ("effectful", true)] {
        let file = parse_source(
            "function declared() { opaque(); } declared();",
            Path::new("scoped.js"),
            parse_declarations(&format!(
                "fn declared() {{ call_effects = {call_effects} returns void }}"
            ))
            .unwrap(),
        )
        .unwrap();
        analysis.add_file(key, Arc::new(file)).unwrap();
    }
    let result = analysis
        .solve(&|_: &&str, _: ExternalReferenceView<'_>| Ok(Resolution::Unknown))
        .unwrap();
    assert!(!result
        .evaluation
        .effect(result.files["pure"].roots[1].effects));
    assert!(result
        .evaluation
        .effect(result.files["effectful"].roots[1].effects));
}

#[test]
fn parses_node_contracts_and_document_examples() {
    let facts = parse_declarations(DECLARATIONS).unwrap();
    assert_eq!(facts.definitions().len(), 6);
    assert_eq!(facts.definitions()[0].returns(), &Returns::Void);
    assert!(facts.definitions()[0].variadic());
    assert_eq!(
        facts.definitions()[4].binding(),
        &Binding::Module {
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
        "global symbol ambient {}",
        "global symbol ambient { read_effects = maybe }",
        "global symbol ambient { call_effects = true }",
        "global symbol ambient { read_effects = false read_effects = true }",
        "global symbol ambient { read_effects = true } global symbol ambient { read_effects = false }",
        "symbol ambient { read_effects = true }",
    ] {
        assert!(parse_declarations(text).is_err(), "accepted {text}");
    }
    let error = parse_declarations("fn f() { @ }").unwrap_err().to_string();
    assert!(error.contains("byte 9"), "{error}");
}

#[test]
fn global_symbol_declarations_validate_owned_facts() {
    let facts = parse_declarations(
        "global symbol ambient { read_effects=true } global symbol ordinary { read_effects=false }",
    )
    .unwrap();
    insta::assert_debug_snapshot!(facts.global_symbols(), @r#"
    [
        GlobalSymbol {
            name: "ambient",
            read_effects: true,
        },
        GlobalSymbol {
            name: "ordinary",
            read_effects: false,
        },
    ]
    "#);
    Context::from_local_facts(facts.clone()).unwrap();
    let mut invalid = facts.clone();
    invalid.global_symbols_mut()[0] =
        GlobalSymbol::new(String::new(), facts.global_symbols()[0].read_effects());
    assert!(Context::from_local_facts(invalid).is_err());
    let mut invalid = facts.clone();
    invalid
        .global_symbols_mut()
        .push(facts.global_symbols()[0].clone());
    assert!(Context::from_local_facts(invalid).is_err());
}

#[test]
fn callable_parameter_names_and_owned_fact_validation() {
    let facts = parse_declarations("fn check(call) { call_effects = call.call_effects }").unwrap();
    let mut invalid = facts.clone();
    invalid.definitions_mut()[0].parameters_mut().clear();
    assert!(Context::from_local_facts(invalid).is_err());
    let mut context = Context::from_local_facts(facts).unwrap();
    let argument = context.value(Hazards::NONE);
    let result = context.call(local(&context, "check"), &[argument], None);
    assert!(!context.evaluate().effect(result.effects()));
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
    assert!(!evaluation.effect(ordinary.effects()));
    assert_eq!(evaluation.hazards(ordinary.value()), Hazards::UNKNOWN);
    assert!(!evaluation.effect(defaults.effects()));
    assert!(!evaluation
        .hazards(defaults.value())
        .get(Hazard::PropertyRead));
    assert!(evaluation
        .hazards(defaults.value())
        .get(Hazard::ToPrimitive));
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
    assert!(!evaluation.effect(safe.effects()));
    assert!(!evaluation.effect(missing.effects()));
    assert_eq!(evaluation.hazards(safe.value()), Hazards::NONE);
    let hazardous = context.call(function, &[context.unknown()], None);
    let evaluation = context.evaluate();
    assert!(evaluation.effect(hazardous.effects()));
    assert!(
        evaluation.effect(safe.effects()),
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
    assert_eq!(alias.value(), object);
    let read = context.hazard(alias.value(), Hazard::PropertyRead);
    let dependent = context.any([read, alias.effects()]);
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
    assert_ne!(first.value(), second.value());
    context.update(first.value(), Hazard::PropertyRead, context.effectful());
    let evaluation = context.evaluate();
    assert!(evaluation.hazards(first.value()).get(Hazard::PropertyRead));
    assert!(!evaluation.hazards(second.value()).get(Hazard::PropertyRead));
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
        !evaluation.effect(result.effects()),
        "installing a getter must not execute it"
    );
    assert_eq!(object, result.value());
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
    assert!(!context.evaluate().effect(safe.effects()));
    let unsafe_call = context.call(
        local(&context, "explicit"),
        &[convert, context.unknown()],
        None,
    );
    let evaluation = context.evaluate();
    assert!(evaluation.effect(unsafe_call.effects()));
}

#[test]
fn recursive_summaries_converge_and_propagate_new_hazards() {
    let mut context = context("fn recursive(f) { call_effects = call(f, f) }");
    let function = local(&context, "recursive");
    let result = context.call(function, &[function], None);
    assert!(!context.evaluate().effect(result.effects()));
    context.update(function, Hazard::Call, context.effectful());
    assert!(context.evaluate().effect(result.effects()));
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
        .map(|matched| format!("{:?}", matched.binding()))
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
        assert!(
            analysis
                .matches
                .iter()
                .all(|matched| matches!(matched.binding(), &Binding::Local(_))),
            "{javascript}"
        );
    }
}

fn inferred<'s>(analysis: &'s SourceAnalysis, name: &str) -> &'s Definition {
    analysis
        .context
        .facts()
        .definitions()
        .iter()
        .find(|definition| *definition.binding() == Binding::Local(name.into()))
        .unwrap()
}

#[test]
fn isolated_inference_produces_contracts_independent_of_callers() {
    let declarations = r#"
            global fn Array.push(target, value) { call_effects = true returns void }
            global fn myhelper(value) { call_effects = false returns.*_effects = false }
        "#;
    let javascript = r#"
            function user_function(a) {
                Array.push(a, "hello");
                return myhelper(a);
            }
        "#;
    let standalone = source(declarations, javascript);
    let called = source(
        declarations,
        &format!("{javascript} user_function(unknown); user_function(1);"),
    );
    let definition = inferred(&standalone, "user_function");
    assert_eq!(definition, inferred(&called, "user_function"));
    insta::assert_debug_snapshot!((
            &definition.binding(), &definition.parameters(),
            definition.hazards().get(Hazard::Call), &definition.returns(),
            definition.return_hazards().get(Hazard::PropertyRead),
            &definition.updates(),
        ), @r#"
        (
            Local(
                "user_function",
            ),
            [
                "a",
            ],
            Bool(
                true,
            ),
            Fresh,
            Bool(
                false,
            ),
            [],
        )
        "#);
}

#[test]
fn isolated_parameter_hazards_and_return_aliases_apply_at_calls() {
    let analysis = source(
        DECLARATIONS,
        r#"
            function read(value) { return value.field; }
            function convert(value) { return Number(value); }
            function identity(value) { const alias = value; return alias; }
            const double = value => value + value;
            const object = {};
            identity(object);
            read(object);
            convert(1);
            double(2);
        "#,
    );
    assert_eq!(effects(&analysis), vec![false; 9]);
    assert_eq!(analysis.roots[4].value, analysis.roots[5].value);
    assert_eq!(
        inferred(&analysis, "read").hazards().get(Hazard::Call),
        HazardExpr::Read {
            value: "value".into(),
            slot: Hazard::PropertyRead
        }
    );
    assert_eq!(
        inferred(&analysis, "convert").hazards().get(Hazard::Call),
        HazardExpr::Read {
            value: "value".into(),
            slot: Hazard::ToPrimitive
        }
    );
    assert_eq!(
        inferred(&analysis, "identity").returns(),
        &Returns::Argument("value".into())
    );
    assert_eq!(
        inferred(&analysis, "double")
            .hazards()
            .get(Hazard::Construct),
        HazardExpr::Bool(false)
    );
    let unsafe_read = source(
        "",
        "function read(value) { return value.field; } read(unknown);",
    );
    assert_eq!(effects(&unsafe_read), vec![false, true]);
}

#[test]
fn isolated_dependencies_compose_without_source_order_or_caller_values() {
    let analysis = source(
        DECLARATIONS,
        r#"
            function first(input) { return second(input); }
            const second = input => Number(input);
            first(1);
        "#,
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(
        inferred(&analysis, "first").hazards().get(Hazard::Call),
        HazardExpr::Read {
            value: "input".into(),
            slot: Hazard::ToPrimitive
        }
    );
    let imported = source(
        DECLARATIONS,
        r#"
            import { join as combine } from "node:path";
            const wrapper = input => combine(input);
            wrapper("hello");
        "#,
    );
    assert_eq!(effects(&imported), vec![false; 2]);
    let overridden = source(
        "fn supplied() { call_effects = false returns void }",
        "function supplied() { unknown(); } supplied();",
    );
    assert_eq!(effects(&overridden), vec![false; 2]);
    let exported = source(
        "",
        r#"
        export function named(value) { return value; }
        export default function fallback(value) { return named(value); }
        export const expression = function internal(value) { return fallback(value); };
        expression(1);
    "#,
    );
    assert_eq!(effects(&exported), vec![false; 4]);
    for name in ["named", "fallback", "expression"] {
        assert_eq!(
            inferred(&exported, name).returns(),
            &Returns::Argument("value".into())
        );
    }
}

#[test]
fn isolated_callbacks_remain_symbolic_and_specialize_in_contract_solver() {
    let analysis = source(
        "",
        r#"
            function invoke(callback, value) { return callback(value); }
            function read(input) { return input.field; }
            invoke(read, {});
        "#,
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(
        inferred(&analysis, "invoke").hazards().get(Hazard::Call),
        HazardExpr::Call {
            function: "callback".into(),
            arguments: vec![CallArgument::Value("value".into())],
            receiver: None
        }
    );
    assert_eq!(inferred(&analysis, "invoke").returns(), &Returns::Unknown);
}

#[test]
fn isolated_known_updates_and_receiver_dependencies_are_composed() {
    let declarations = r#"
            global fn mutate(value) {
                call_effects = true
                updates value.property_read_effects |= true
                returns = value
            }
            global fn Number(value) { call_effects = value.toprimitive_effects returns.*_effects = false }
            global fn install(target, getter) {
                call_effects = false
                updates target.property_read_effects |= call(getter, this = target)
                returns = target
            }
        "#;
    let analysis = source(
        declarations,
        r#"
            function wrapper(input) { return mutate(input); }
            function getter() { return Number(this); }
            function register(target) { return install(target, getter); }
            const target = {};
            register(target);
            target.field;
        "#,
    );
    assert_eq!(effects(&analysis), vec![false; 6]);
    let wrapper = inferred(&analysis, "wrapper");
    assert_eq!(wrapper.returns(), &Returns::Argument("input".into()));
    assert_eq!(
        wrapper.updates(),
        vec![Update::new(
            "input".into(),
            Some(Hazard::PropertyRead),
            HazardExpr::Bool(true)
        )]
    );
    assert_eq!(
        inferred(&analysis, "register").updates(),
        vec![Update::new(
            "target".into(),
            Some(Hazard::PropertyRead),
            HazardExpr::Read {
                value: "target".into(),
                slot: Hazard::ToPrimitive
            }
        )]
    );
}

#[test]
fn isolated_callbacks_propagate_known_updates_without_executing_queries() {
    let analysis = source(
        r#"
            global fn mutate(value) { call_effects = false updates value.property_read_effects |= true returns void }
            global fn invoke(callback, value) { call_effects = call(callback, value) returns void }
        "#,
        r#"
            function change(input) { mutate(input); }
            function wrapper(target) { invoke(change, target); }
        "#,
    );
    assert_eq!(effects(&analysis), vec![false; 2]);
    assert_eq!(
        inferred(&analysis, "wrapper").updates(),
        vec![Update::new(
            "target".into(),
            Some(Hazard::PropertyRead),
            HazardExpr::Bool(true)
        )]
    );
}

#[test]
fn isolated_returns_join_conservatively_without_live_identity_tracking() {
    let analysis = source(
        "",
        r#"
            function choose(flag, left, right) { if (flag) return left; return right; }
            function fresh(flag) { if (flag) return {}; return 1; }
            function empty() {}
            function stopped() { return 1; unknown(); }
        "#,
    );
    assert_eq!(inferred(&analysis, "choose").returns(), &Returns::Unknown);
    assert_eq!(inferred(&analysis, "fresh").returns(), &Returns::Fresh);
    assert_eq!(inferred(&analysis, "empty").returns(), &Returns::Void);
    assert_eq!(
        inferred(&analysis, "stopped").hazards().get(Hazard::Call),
        HazardExpr::Bool(false)
    );
}

#[test]
fn isolated_unsupported_bodies_and_cycles_are_conservative_but_deferred() {
    for function in [
        "function f(value) { let alias = value; return alias; }",
        "function f(value) { value.field = 1; return value; }",
        "function f(value) { while (value) unknown(); }",
        "function f(...values) { return values; }",
        "function f({ value }) { return value; }",
        "async function f(value) { return value; }",
        "function f(value) { return f(value); }",
    ] {
        let uncalled = source("", &format!("{function} +1;"));
        assert_eq!(effects(&uncalled), vec![false; 2], "{function}");
        assert_eq!(
            inferred(&uncalled, "f").hazards().get(Hazard::Call),
            HazardExpr::Unknown,
            "{function}"
        );
        assert_eq!(
            inferred(&uncalled, "f").returns(),
            &Returns::Unknown,
            "{function}"
        );
        let mut context = Context::from_local_facts(uncalled.context.facts().clone()).unwrap();
        let argument = context.value(Hazards::NONE);
        let function = context.definition(&Binding::Local("f".into())).unwrap();
        let called = context.call(function, &[argument], None);
        assert!(context.evaluate().effect(called.effects()));
    }
}

#[test]
fn isolated_contracts_respect_lexical_shadows() {
    let analysis = source(
        DECLARATIONS,
        r#"
            function Number(value) { unknown(value); }
            function wrapper(value) { return Number(value); }
            function parameter(Number, value) { return Number(value); }
        "#,
    );
    assert_eq!(
        inferred(&analysis, "wrapper").hazards().get(Hazard::Call),
        HazardExpr::Unknown
    );
    assert_eq!(
        inferred(&analysis, "parameter").hazards().get(Hazard::Call),
        HazardExpr::Call {
            function: "Number".into(),
            arguments: vec![CallArgument::Value("value".into())],
            receiver: None
        }
    );
}

#[test]
fn isolated_contract_growth_is_bounded_for_shared_dependencies_and_large_bodies() {
    let mut javascript = "function f0(input) { return input.value; }\n".to_owned();
    for index in 1..1000 {
        javascript.push_str(&format!(
            "function f{index}(input) {{ f{}(input); return f{}(input); }}\n",
            index - 1,
            index - 1
        ));
    }
    let analysis = source("", &javascript);
    assert_eq!(analysis.context.facts().definitions().len(), 1000);
    assert_eq!(
        inferred(&analysis, "f999").hazards().get(Hazard::Call),
        HazardExpr::Read {
            value: "input".into(),
            slot: Hazard::PropertyRead
        }
    );
    let large = source(
        "",
        &format!(
            "function large(value) {{ {} return value; }}",
            "value.field;".repeat(5000)
        ),
    );
    assert_eq!(
        inferred(&large, "large").hazards().get(Hazard::Call),
        HazardExpr::Unknown
    );
    let parameters = (0..80)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut growing = format!("function growing({parameters}) {{");
    let mut previous = "p0".to_owned();
    for index in 1..80 {
        growing.push_str(&format!("const v{index} = merge({previous}, p{index});"));
        previous = format!("v{index}");
    }
    growing.push_str(&format!("return {previous}; }}"));
    let expanded = source(
        r#"
        global fn merge(left, right) {
            call_effects = false
            returns.*_effects = left.property_read_effects | right.property_read_effects
        }
    "#,
        &growing,
    );
    assert_eq!(
        inferred(&expanded, "growing").hazards().get(Hazard::Call),
        HazardExpr::Unknown
    );
    assert_eq!(inferred(&expanded, "growing").returns(), &Returns::Unknown);
}

#[test]
fn isolated_contract_api_returns_owned_facts_without_an_analysis_context() {
    use oxc_allocator::Allocator;
    use oxc_semantic::SemanticBuilder;
    let inferred = {
        let allocator = Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            "function identity(value) { return value; }",
            Path::new("input.js"),
            oxc_utils_parse::ModuleFormat::Esm,
        )
        .unwrap();
        let semantic = SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        infer_function_contracts(&parsed.program, &semantic.semantic, &LocalFacts::default())
            .unwrap()
    };
    assert_eq!(
        inferred.definitions()[0].returns(),
        &Returns::Argument("value".into())
    );
    Context::from_local_facts(inferred).unwrap();
}

#[test]
fn installed_contract_lookup_preserves_first_file_binding() {
    let first = parsed("function identity(value) { return value; }");
    let second = parsed("function identity(value) { return value; }");
    let id = symbol(&first, "identity");
    let mut analysis = Analysis::new();
    analysis.add_file("first", first).unwrap();
    analysis.add_file("second", second).unwrap();
    let result = analysis
        .solve(&|_: &&str, _: ExternalReferenceView<'_>| Ok(Resolution::Unknown))
        .unwrap();
    let value = result
        .context
        .definition(&Binding::Local("identity".into()))
        .unwrap();
    assert_eq!(value, result.files["first"].symbols[&id]);
    assert_ne!(value, result.files["second"].symbols[&id]);
    let single = source(
        "fn supplied() { call_effects = false returns void }",
        "supplied();",
    );
    assert!(single
        .context
        .definition(&Binding::Local("supplied".into()))
        .is_some());
}

#[test]
fn commonjs_duplicate_parameters_produce_valid_unknown_contracts() {
    let parsed = parse_source(
        "function repeated(value, value) { return value; }",
        Path::new("input.cjs"),
        LocalFacts::default(),
    )
    .unwrap();
    parsed.validate().unwrap();
    assert_eq!(
        parsed.declarations().definitions()[0]
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Unknown
    );
}

#[test]
fn cached_contracts_preserve_redeclarations_getters_pure_and_nested_shadows() {
    let analysis = source(
        "global symbol ambient { read_effects = true }",
        "function read() { return ambient; } function unused() { /*@__PURE__*/ unknown(); }",
    );
    assert_eq!(
        inferred(&analysis, "read").hazards().get(Hazard::Call),
        HazardExpr::Unknown
    );
    assert_eq!(
        inferred(&analysis, "unused").hazards().get(Hazard::Call),
        HazardExpr::Bool(false)
    );
    let redeclared = parse_source(
        "function changed() { return 1; } function changed() { unknown(); } changed();",
        Path::new("redeclaration.cjs"),
        LocalFacts::default(),
    )
    .unwrap();
    assert_eq!(
        redeclared.declarations().definitions()[0]
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Unknown
    );
    let analysis = source(
        "",
        "const identity = value => value; function outer() { const identity = () => unknown(); identity(); } outer();",
    );
    assert!(effects(&analysis).last().unwrap());
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
    assert!(context.evaluate().effect(result.effects()));
}

#[test]
fn unknown_callback_summaries_widen_mutations() {
    let mut context = context("fn invoke(callback) { call_effects = call(callback) }");
    let victim = context.value(Hazards::NONE);
    let result = context.call(local(&context, "invoke"), &[context.unknown()], None);
    let evaluation = context.evaluate();
    assert!(evaluation.effect(result.effects()));
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
        import { condition } from "unknown";
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
    let parsed = oxc_utils_parse::parse_ts(
        &allocator,
        "const a: number = 1; Number(a); /*#__PURE__*/ unknown();",
    );
    assert!(parsed.diagnostics.is_empty());
    let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
    assert!(semantic.diagnostics.is_empty());
    let analysis = extract_from_ast(
        &parsed.program,
        &semantic.semantic,
        parse_declarations(DECLARATIONS).unwrap(),
    )
    .unwrap();
    assert_eq!(effects(&analysis), vec![false; 3]);
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
    let result = analysis.context.call(closure.value(), &[], Some(other));
    assert_eq!(result.value(), receiver);
    let evaluation = analysis.context.evaluate();
    assert!(!evaluation.effect(closure.effects()));
    assert!(!evaluation.effect(result.effects()));
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

#[test]
fn astra_declared_call_updates_invalidate_inferred_calls() {
    let analysis = source(
        r#"
        global fn replace(callback) {
            call_effects = false
            updates callback.call_effects |= true
            returns void
        }
    "#,
        r#"
        const callback = () => 1;
        callback();
        replace(callback);
        callback();
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, true, true, true],
        "executing a replaced callback invalidates specialized calls and may replace other targets"
    );
}

#[test]
fn astra_inferred_rebinding_preserves_saved_callable_identity() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function save(callback) {
            const saved = callback;
            var callback = () => 1;
            return saved;
        }
        const original = () => console.log("must run");
        const saved = save(original);
        saved();
    "#,
    );
    assert!(
        *effects(&analysis).last().unwrap(),
        "rebinding a parameter must not redirect a saved alias to a different, pure function"
    );
    assert_eq!(analysis.roots[1].value, analysis.roots[2].value);
}

#[test]
fn astra_declared_bare_call_hazards_are_unspecialized() {
    let analysis = source(
        r#"
        global fn read(value) {
            call_effects = value.property_read_effects
            returns void
        }
        global fn probe(callback) {
            call_effects = callback.call_effects
            returns void
        }
    "#,
        "probe(read);",
    );
    assert_eq!(
        effects(&analysis),
        vec![true],
        "a bare hazard query cannot assume that read will only receive undefined"
    );
}

#[test]
fn astra_inferred_branch_bindings_preserve_effectful_alternatives() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function choose(flag) {
            if (flag) {
                var callback = () => console.log("must run");
            } else {
                var callback = () => 1;
            }
            return callback;
        }
        const callback = choose(true);
        callback();
    "#,
    );
    assert!(
        *effects(&analysis).last().unwrap(),
        "an alternate branch must not overwrite the callable selected by the consequent"
    );
}

#[test]
fn astra_latent_unknown_callback_does_not_poison_unrelated_reads() {
    let analysis = source(
        r#"
        global fn install(target, callback) {
            call_effects = false
            updates target.property_read_effects |= call(callback)
            returns void
        }
    "#,
        r#"
        import { unknownCallback } from "unknown";
        const unrelated = {};
        unrelated.value;
        const target = {};
        install(target, unknownCallback);
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false; 4],
        "describing a future getter must not execute an unknown callback"
    );
    let evaluation = analysis.context.evaluate();
    assert!(evaluation
        .hazards(analysis.roots[2].value)
        .get(Hazard::PropertyRead));
}

#[test]
fn astra_round2_hoisted_function_reads_expose_callable_hazards() {
    let analysis = source(
        r#"
        global fn probe(callback) {
            call_effects = callback.call_effects
            returns void
        }
    "#,
        r#"
        function callback(value) { return value.property; }
        probe(callback);
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, true],
        "a hoisted function's lexical slot must not hide its unspecialized call hazard"
    );
}

#[test]
fn astra_round2_uninitialized_var_redeclaration_preserves_parameter() {
    let analysis = source(
        "",
        r#"
        function keep(value) {
            var value;
            return value;
        }
        const original = {};
        const saved = keep(original);
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(
        analysis.roots[1].value, analysis.roots[2].value,
        "a var declaration without an initializer does not overwrite its parameter"
    );
    let analysis = source("", "var value; const initial = value; var value;");
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert!(analysis
        .roots
        .iter()
        .all(|root| root.value == analysis.context.undefined()));
}

#[test]
fn astra_round2_callable_join_does_not_invent_replacement_mutations() {
    let mut analysis = source(
        "",
        r#"
        const read = value => value.property;
        const other = () => 1;
        const selected = true ? read : other;
        read({});
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false; 4],
        "joining function identities must not turn a bare call hazard into a mutation of read"
    );
    let selected = analysis.roots[2].value;
    analysis
        .context
        .update(selected, Hazard::Call, analysis.context.effectful());
    assert_eq!(effects(&analysis), vec![false, false, false, true]);
}

#[test]
fn astra_round2_nested_declared_callback_execution_applies_updates() {
    let analysis = source(
        r#"
        global fn invoke(callback, ...args) {
            call_effects = call(callback, ...args)
            returns void
        }
        global fn mutate(target) {
            call_effects = false
            updates target.property_read_effects |= true
            returns void
        }
    "#,
        r#"
        const target = {};
        invoke(invoke, mutate, target);
        target.property;
    "#,
    );
    assert!(
        effects(&analysis)[2],
        "executing invoke(invoke, mutate, target) must propagate mutate's property-read update"
    );
}

#[test]
fn astra_round3_false_update_on_unknown_does_not_poison() {
    let analysis = source(
        r#"
        global fn noop(target) {
            call_effects = false
            updates target.property_read_effects |= false
            returns void
        }
    "#,
        r#"
        import { unknownValue } from "unknown";
        const unrelated = {};
        noop(unknownValue);
        unrelated.property;
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false; 3],
        "merging a false hazard into an unknown target is not unknown execution or a mutation"
    );
}

#[test]
fn astra_round3_self_loose_equality_does_not_coerce() {
    let declarations = r#"
        global fn object() {
            call_effects = false
            returns.*_effects = false
            returns.toprimitive_effects = true
        }
    "#;
    let analysis = source(
        declarations,
        "const value = object(); value == value; value != value;",
    );
    assert_eq!(
        effects(&analysis),
        vec![false; 3],
        "loose equality of a value with itself never invokes ToPrimitive, even with effectful hooks"
    );
    let analysis = source(
        declarations,
        "function equal(x, y) { return x == y; } const value = object(); equal(value, value);",
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(
        effects(&source("", "unknownLeft == unknownRight;")),
        vec![true]
    );
}

#[test]
fn astra_round3_mutating_join_with_unknown_widens_external_aliases() {
    let mut context = context("");
    let unrelated = context.value(Hazards::NONE);
    let known = context.value(Hazards::NONE);
    let target = context.join_values(&[known, context.unknown()]);
    let read = context.hazard(unrelated, Hazard::PropertyRead);
    let signal = context.value(Hazards::NONE);
    let mutation = context.hazard(signal, Hazard::Call);
    context.update(target, Hazard::PropertyRead, mutation);
    assert!(!context.evaluate().effect(read));
    context.update(signal, Hazard::Call, context.effectful());
    for workers in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        assert!(
            pool.install(|| context.evaluate()).effect(read),
            "a may-alias join containing unknown must retain unknown's potential aliases when mutated"
        );
    }
}

#[test]
fn astra_round4_returned_closure_preserves_terminated_branch_capture() {
    let analysis = source(
        DECLARATIONS,
        r#"
        function choose(flag) {
            var callback = () => console.log("must run");
            const saved = () => callback();
            if (flag) return saved;
            var callback = () => 1;
            return saved;
        }
        const selected = choose(true);
        selected();
    "#,
    );
    assert!(
        *effects(&analysis).last().unwrap(),
        "the continuing branch must not overwrite a capture belonging to the returned branch"
    );
}

#[test]
fn astra_round4_latent_declared_callback_accounts_for_its_updates() {
    let mut context = context(
        r#"
        fn mutate_and_read(target) {
            call_effects = target.property_read_effects
            updates target.property_read_effects |= true
            returns void
        }
        fn install(holder, callback, input) {
            call_effects = false
            updates holder.property_read_effects |= call(callback, input)
            returns void
        }
    "#,
    );
    let input = context.value(Hazards::NONE);
    let holder = context.value(Hazards::NONE);
    context.call(
        local(&context, "install"),
        &[holder, local(&context, "mutate_and_read"), input],
        None,
    );
    let evaluation = context.evaluate();
    assert!(!evaluation.hazards(input).get(Hazard::PropertyRead));
    assert!(
        evaluation.hazards(holder).get(Hazard::PropertyRead),
        "latent declared callbacks must account for hypothetical writes without applying them"
    );
}

#[test]
fn astra_round4_deferred_namespace_reads_share_global_identity() {
    let analysis = source(
        r#"
        global fn library.method() {
            call_effects = false
            returns void
        }
        global fn install(target) {
            call_effects = false
            updates target.property_read_effects |= true
            returns void
        }
    "#,
        r#"
        function use_library() { library.method(); }
        install(library);
        use_library();
    "#,
    );
    assert!(
        effects(&analysis)[2],
        "installing a getter on the global namespace must invalidate deferred member reads"
    );
}

#[test]
fn astra_round4_replaced_call_target_invalidates_result_hazards() {
    let analysis = source(
        r#"
        global fn replace(callback) {
            call_effects = false
            updates callback.call_effects |= true
            returns void
        }
    "#,
        r#"
        const make = () => ({});
        replace(make);
        const result = make();
        result.property;
    "#,
    );
    assert!(effects(&analysis)[2]);
    assert!(
        effects(&analysis)[3],
        "executing a replaced callable must not retain the original pure result summary"
    );
}

#[test]
fn astra_round4_inferred_constructor_exposes_construct_hazard() {
    let declarations = r#"
        global fn construct(constructor) {
            call_effects = constructor.construct_effects
            returns void
        }
        global fn log() {
            call_effects = true
            returns void
        }
    "#;
    let analysis = source(
        declarations,
        r#"
        function Constructor() { log(); }
        construct(Constructor);
    "#,
    );
    assert_eq!(
        effects(&analysis),
        vec![false, true],
        "an ordinary function's construct hazard must account for execution of its body"
    );
    let analysis = source(declarations, "const Arrow = () => log(); construct(Arrow);");
    assert_eq!(effects(&analysis), vec![false; 2]);
}

#[test]
fn astra_round5_ambient_identifier_reads_can_execute_global_getters() {
    let cases = [
        "ambient;",
        "typeof ambient;",
        "function read() { return ambient; } read();",
        "function read() { return typeof ambient; } read();",
        "function read() { return ambient; } function helper() { return read(); } helper();",
        "function read() { return typeof ambient; } function helper() { return read(); } helper();",
    ];
    for (declarations, expected) in [
        ("", false),
        ("global symbol ambient { read_effects = false }", false),
        ("global symbol ambient { read_effects = true }", true),
    ] {
        let observed: Vec<_> = cases
            .into_iter()
            .map(|javascript| *effects(&source(declarations, javascript)).last().unwrap())
            .collect();
        assert_eq!(observed, vec![expected; cases.len()], "{declarations}");
    }
}

#[test]
fn global_symbol_reads_respect_lexical_bindings_and_callable_contracts() {
    let declarations = "global symbol ambient { read_effects = true }";
    for javascript in [
        "const ambient = 1; ambient; typeof ambient;",
        "function read(ambient) { return ambient; } read(1);",
        "import { ambient } from 'unknown'; ambient;",
    ] {
        let analysis = source(declarations, javascript);
        assert!(
            effects(&analysis).iter().all(|effect| !effect),
            "{javascript}"
        );
        assert!(
            !analysis
                .matches
                .iter()
                .any(|matched| matches!(matched.binding(), &Binding::Global(_))),
            "{javascript}"
        );
    }
    let analysis = source(
        "global symbol ambient { read_effects = false } global fn ambient() { call_effects = false returns void }",
        "ambient; ambient();",
    );
    assert_eq!(effects(&analysis), vec![false; 2]);
    assert_eq!(analysis.matches.len(), 4);
    let analysis = source(
        "global symbol library { read_effects = true } global fn library.method() { call_effects = false returns void }",
        "library.method();",
    );
    assert_eq!(effects(&analysis), vec![true]);
}

#[test]
fn global_symbol_getters_are_deferred_and_latent_until_execution() {
    let declarations = r#"
        global symbol ambient { read_effects = true }
        global fn install(holder, getter) {
            call_effects = false
            updates holder.property_read_effects |= call(getter)
            returns void
        }
    "#;
    let analysis = source(
        declarations,
        "function unused() { return ambient; } const object = {}; object.property;",
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(analysis.matches.len(), 1);
    let analysis = source(
        declarations,
        "const object = {}; object.property; const holder = {}; install(holder, () => ambient);",
    );
    assert_eq!(effects(&analysis), vec![false; 4]);
    assert!(analysis
        .context
        .evaluate()
        .hazards(analysis.roots[2].value)
        .get(Hazard::PropertyRead));
    let analysis = source(
        declarations,
        "const object = {}; object.property; ambient; object.property;",
    );
    assert_eq!(effects(&analysis), vec![false, true, true, true]);
}

#[test]
fn astra_round5_latent_callback_tracks_later_capture_rebinding() {
    let declarations = r#"
        global fn install(holder, getter) {
            call_effects = false
            updates holder.property_read_effects |= call(getter)
            returns void
        }
        global fn invoke(callback) {
            call_effects = call(callback)
            returns void
        }
        global fn log() { call_effects = true returns void }
    "#;
    let observed: Vec<_> = [
        r#"
        var callback = () => 1;
        const getter = () => callback();
        const holder = {};
        install(holder, getter);
        var callback = () => log();
        holder.property;
    "#,
        r#"
        var target = () => 1;
        const callback = () => target();
        invoke(callback);
        var target = () => log();
        invoke(callback);
    "#,
    ]
    .into_iter()
    .map(|javascript| *effects(&source(declarations, javascript)).last().unwrap())
    .collect();
    assert_eq!(
        observed,
        vec![true; 2],
        "latent getters and cached callback effects must observe later changes to live captures"
    );
}

#[test]
fn astra_round6_opaque_capture_rebinding_invalidates_equality_identity() {
    let analysis = source(
        "global fn log() { call_effects = true returns void }",
        r#"
        let current = { valueOf() { log(); return 1; } };
        const saved = current;
        function change() { current = 1; }
        change();
        saved == current;
    "#,
    );
    assert!(
        *effects(&analysis).last().unwrap(),
        "an opaque write can separate previously identical bindings, making equality invoke valueOf"
    );
}

#[test]
fn astra_round7_implicit_arguments_shadows_global_accessor() {
    let declarations = "global symbol arguments { read_effects = true }";
    for javascript in [
        "function own() { return arguments; } own();",
        "function own() { return (() => arguments)(); } own();",
        "function own() { return (() => (() => arguments)())(); } own();",
    ] {
        let analysis = source(declarations, javascript);
        assert_eq!(
            effects(&analysis),
            vec![false; 2],
            "the ordinary function's implicit arguments binding shadows the global accessor"
        );
        assert!(analysis.matches.is_empty());
    }
    assert_eq!(
        effects(&source(declarations, "(() => arguments)();")),
        vec![true]
    );
    let analysis = source(
        "",
        "function own() { return arguments; } const a = own(); const b = own();",
    );
    assert_ne!(analysis.roots[1].value, analysis.roots[2].value);
}

#[test]
fn astra_round7_instanceof_checks_left_prototype_traps() {
    let analysis = source(
        r#"
        global fn makeProxy() {
            call_effects = false
            returns.property_read_effects = false
        }
    "#,
        r#"
        function Constructor() {}
        const proxy = makeProxy();
        proxy instanceof Constructor;
    "#,
    );
    assert!(
        *effects(&analysis).last().unwrap(),
        "a proxy with ordinary property reads can still execute an effectful getPrototypeOf trap"
    );
    for (hazards, expected) in [
        (
            "returns.*_effects = false returns.property_read_effects = true",
            false,
        ),
        (
            "returns.*_effects = false returns.prototype_read_effects = true",
            true,
        ),
    ] {
        let analysis = source(
            &format!("global fn object() {{ call_effects = false {hazards} }}"),
            "function Constructor() {} object() instanceof Constructor;",
        );
        assert_eq!(*effects(&analysis).last().unwrap(), expected, "{hazards}");
    }
}

#[test]
fn pure_annotations_override_invocation_effects_without_poisoning_the_heap() {
    for marker in ["/*#__PURE__*/", "/*@__PURE__*/"] {
        for invocation in [
            "unknown()",
            "(unknown())",
            "unknown?.()",
            "new Unknown()",
            "new Unknown",
            "console.log(1)",
            "(() => unknown())()",
        ] {
            let analysis = source(
                DECLARATIONS,
                &format!("const object = {{}}; object.value; {marker} {invocation}; object.value;"),
            );
            assert_eq!(effects(&analysis), vec![false; 4], "{marker} {invocation}");
        }
    }
    for javascript in ["unknown();", "/* PURE */ unknown();", "new Unknown();"] {
        assert_eq!(effects(&source("", javascript)), vec![true], "{javascript}");
    }
}

#[test]
fn pure_annotations_preserve_callee_argument_and_iteration_effects() {
    for invocation in [
        "unknown(console.log(1))",
        "new Unknown(console.log(1))",
        "object.method()",
        "unknown(...items)",
        "new Unknown(...items)",
    ] {
        let analysis = source(DECLARATIONS, &format!("/*#__PURE__*/ {invocation};"));
        assert_eq!(effects(&analysis), vec![true], "{invocation}");
    }
    let analysis = source(
        "global symbol ambient { read_effects = true }",
        "/*#__PURE__*/ ambient();",
    );
    assert_eq!(effects(&analysis), vec![true]);
    for invocation in ["unknown(...items())", "new Unknown(...items())"] {
        let analysis = source(
            "global fn items() { call_effects = false returns.iterate_effects = false }",
            &format!("/*#__PURE__*/ {invocation};"),
        );
        assert_eq!(effects(&analysis), vec![false], "{invocation}");
    }
}

#[test]
fn pure_calls_in_summaries_skip_updates_and_preserve_known_returns() {
    let declarations = r#"
        global fn mutate(target) {
            call_effects = true
            updates target.property_read_effects |= true
            returns = target
        }
    "#;
    let analysis = source(
        declarations,
        r#"
        function wrap(target) { return /*#__PURE__*/ mutate(target); }
        const object = {};
        object.value;
        const alias = wrap(object);
        alias.value;
    "#,
    );
    assert_eq!(effects(&analysis), vec![false; 5]);
    assert_eq!(analysis.roots[1].value, analysis.roots[3].value);
    let analysis = source(
        "",
        "function identity(x) { return x; } const object = {}; const alias = /*#__PURE__*/ identity(object);",
    );
    assert_eq!(effects(&analysis), vec![false; 3]);
    assert_eq!(analysis.roots[1].value, analysis.roots[2].value);
    for invocation in ["unknown()", "new Unknown()"] {
        let analysis = source(
            "",
            &format!("const result = /*#__PURE__*/ {invocation}; result.value;"),
        );
        assert_eq!(effects(&analysis), vec![false, true], "{invocation}");
    }
}

#[test]
fn pure_optional_member_calls_preserve_receiver_and_key_effects() {
    let declarations = r#"
        global fn namespace.method() { call_effects = true returns = this }
        global fn key() { call_effects = true returns.*_effects = false }
    "#;
    for invocation in ["namespace?.method?.()", "(namespace?.method)()"] {
        let analysis = source(
            declarations,
            &format!("const receiver = namespace; const result = /*#__PURE__*/ {invocation};"),
        );
        assert_eq!(effects(&analysis), vec![false; 2], "{invocation}");
        assert_eq!(
            analysis.roots[0].value, analysis.roots[1].value,
            "{invocation}"
        );
    }
    let analysis = source(declarations, "/*#__PURE__*/ namespace?.[key()]?.();");
    assert_eq!(effects(&analysis), vec![true]);
}
