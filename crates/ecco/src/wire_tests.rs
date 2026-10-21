use std::{collections::BTreeMap, path::Path, sync::Arc};

use cache_flatbuffers::{wire as w, WireEcco};
use serde_json::{json, Value};

use crate::{
    solver::{FunctionSignature, Instruction},
    views::*,
    *,
};

fn parse(source: &str, declarations: &str) -> Arc<ParsedFile> {
    Arc::new(
        parse_source(
            source,
            Path::new("wire.js"),
            parse_declarations(declarations).unwrap(),
        )
        .unwrap(),
    )
}

fn wire(file: &ParsedFile) -> Arc<WireEcco> {
    Arc::new(WireEcco::new(file.to_flatbuffer().into()).unwrap())
}

fn function_projection(f: FunctionView<'_>) -> Value {
    let s = f.signature();
    json!({
        "captures": s.captures().iter().collect::<Vec<_>>(),
        "parameters": s.parameters().iter().collect::<Vec<_>>(),
        "receiver": s.receiver(), "self": s.self_binding(), "arguments": s.arguments(),
        "constructible": s.constructible(),
        "instructions": f.instructions().iter().map(|i| format!("{i:?}")).collect::<Vec<_>>(),
        "returns": f.returns().iter().collect::<Vec<_>>(), "effects": f.effects(),
        "sources": f.sources().iter().collect::<Vec<_>>(),
    })
}

fn projection(file: ParsedFileView<'_>) -> Value {
    json!({
        "version": file.version(),
        "definitions": file.declarations().definitions().iter().map(DefinitionView::to_owned).collect::<Vec<_>>(),
        "globals": file.declarations().global_symbols().iter().map(GlobalSymbolView::to_owned).collect::<Vec<_>>(),
        "values": file.values().iter().map(|v| (v.hazards(), v.definition(), v.alias())).collect::<Vec<_>>(),
        "functions": file.functions().iter().map(function_projection).collect::<Vec<_>>(),
        "setup": function_projection(file.setup()),
        "symbols": file.symbols().iter().map(|v| (v.id(), v.name(), (v.span().start, v.span().end), v.value())).collect::<Vec<_>>(),
        "roots": file.roots().iter().map(|v| ((v.span().start, v.span().end), function_projection(v.function()))).collect::<Vec<_>>(),
        "externals": file.external_references().iter().map(|v| (v.binding().to_owned(), (v.span().start, v.span().end))).collect::<Vec<_>>(),
        "matches": file.definition_matches().iter().map(DefinitionMatchView::to_owned).collect::<Vec<_>>(),
    })
}

fn results(result: &AnalysisResult<&str>) -> Value {
    let files: BTreeMap<_, _> = result.files.iter().map(|(key, file)| {
        let symbols: BTreeMap<_, _> = file.symbols.iter().map(|(id, value)| {
            (id.0, (*value, result.evaluation.hazards(*value)))
        }).collect();
        (*key, json!({
            "roots": file.roots.iter().map(|r| ((r.span.start, r.span.end), r.value,
                result.evaluation.hazards(r.value), result.evaluation.effect(r.effects))).collect::<Vec<_>>(),
            "matches": file.matches, "symbols": symbols,
        }))
    }).collect();
    json!(files)
}

#[test]
fn byte_backed_solves_preserve_live_captures_aliases_contracts_and_effects() {
    let declarations = r#"
        global fn query(callback) { call_effects = call(callback) returns void }
        global fn install(value) {
            call_effects = false
            updates value.property_read_effects |= true
            returns = value
        }
    "#;
    for rebind in [false, true] {
        let library = parse(
            &format!(
                "export function read(input) {{ return input.value; }}
             export var callback = () => 1;
             export function make(x) {{ return () => x.value; }}
             export const value = install({{}});
             {}",
                if rebind {
                    "var callback = () => opaque();"
                } else {
                    ""
                }
            ),
            declarations,
        );
        let entry = parse(
            "import { read, callback, make, value } from 'library';
             const input = {}; const alias = input;
             read(input); read({ get value() {} });
             const closure = make(input); closure();
             function capture() { return callback(); } query(capture);
             value.key; /*@__PURE__*/ opaque();",
            declarations,
        );
        let symbol_ids: BTreeMap<_, _> = library
            .symbols()
            .iter()
            .map(|s| (s.name(), s.id()))
            .collect();
        let resolve = |_: &&str, reference: ExternalReferenceView<'_>| {
            Ok(match reference.binding() {
                BindingView::Module {
                    module: "library",
                    path,
                } => Resolution::Symbol(Symbol {
                    file: "library",
                    local: symbol_ids[path.get(0).unwrap()],
                }),
                BindingView::Global(path)
                    if path.iter().eq(["query"]) || path.iter().eq(["install"]) =>
                {
                    Resolution::Declaration(reference.binding().to_owned())
                }
                _ => Resolution::Unknown,
            })
        };
        for reverse in [false, true] {
            let mut expected = None;
            // Owned, mixed and fully wire graphs all use the same interpreter.
            for mode in 0..3 {
                let library_wire = wire(&library);
                let entry_wire = wire(&entry);
                assert_eq!(projection(library.view()), projection(library_wire.view()));
                assert_eq!(projection(entry.view()), projection(entry_wire.view()));
                let library_source: Arc<dyn ParsedFileSource> = if mode > 0 {
                    library_wire.clone()
                } else {
                    library.clone()
                };
                let entry_source: Arc<dyn ParsedFileSource> = if mode > 1 {
                    entry_wire.clone()
                } else {
                    entry.clone()
                };
                let mut files = vec![("library", library_source), ("entry", entry_source)];
                if reverse {
                    files.reverse();
                }
                let mut analysis = Analysis::new();
                for (key, source) in files {
                    analysis.add_file(key, source).unwrap();
                }
                for _ in 0..2 {
                    let result = analysis.clone().solve(&resolve).unwrap();
                    let observed = results(&result);
                    if let Some(expected) = &expected {
                        assert_eq!(&observed, expected);
                    } else {
                        expected = Some(observed);
                    }
                    let entry_result = &result.files["entry"];
                    let input = entry
                        .symbols()
                        .iter()
                        .find(|s| s.name() == "input")
                        .unwrap()
                        .id();
                    let alias = entry
                        .symbols()
                        .iter()
                        .find(|s| s.name() == "alias")
                        .unwrap()
                        .id();
                    assert_eq!(entry_result.symbols[&input], entry_result.symbols[&alias]);
                    assert!(entry_result
                        .roots
                        .iter()
                        .any(|r| result.evaluation.effect(r.effects)));
                    assert!(!result
                        .evaluation
                        .effect(entry_result.roots.last().unwrap().effects));
                    if mode > 1 {
                        // Templates retain their immutable source rather than copying instructions.
                        assert!(Arc::strong_count(&entry_wire) > 2);
                    }
                }
            }
        }
    }
}

#[test]
fn all_instruction_payloads_and_optional_zero_ids_encode_losslessly() {
    let function = OwnedFunction::new(
        FunctionSignature::new(
            vec![ValueId(0), ValueId(2)],
            vec![ValueId(3)],
            Some(ValueId(0)),
            None,
            true,
            Some(ValueId(0)),
        ),
        vec![
            Instruction::Value(
                ValueId(2),
                Hazards([true, false, true, false, true, false, true, false]),
            ),
            Instruction::Any(Effect(3), vec![Effect(0), Effect(2)]),
            Instruction::Equality(Effect(4), ValueId(0), ValueId(2)),
            Instruction::Operation(ValueId(2), Hazard::PrototypeRead),
            Instruction::Update(ValueId(2), Hazard::Construct, Effect(3)),
            Instruction::Read(ValueId(3), ValueId(2)),
            Instruction::Retain(ValueId(2), vec![ValueId(3), ValueId(2)]),
            Instruction::Bind(ValueId(3), ValueId(2)),
            Instruction::Join(ValueId(3), vec![ValueId(0), ValueId(2)]),
            Instruction::Call(
                CallResult::new(ValueId(3), Effect(5)),
                ValueId(2),
                vec![ValueId(0), ValueId(3)],
                ValueId(0),
                true,
            ),
            Instruction::Call(
                CallResult::new(ValueId(3), Effect(6)),
                ValueId(2),
                vec![],
                ValueId(1),
                false,
            ),
            Instruction::Closure(ValueId(2), 0),
            Instruction::External(ValueId(2), 0),
            Instruction::Opaque,
        ],
        vec![ValueId(3), ValueId(0)],
        Some(Effect(0)),
        BTreeMap::from([
            (Effect(2), (ValueId(2), Hazard::Call)),
            (Effect(9), (ValueId(0), Hazard::PrototypeRead)),
        ]),
    );
    let file = ParsedFile::new(
        1,
        LocalFacts::default(),
        vec![
            crate::analysis::ParsedValue::new(Hazards::NONE, None, None),
            crate::analysis::ParsedValue::new(Hazards::UNKNOWN, Some(0), Some(ValueId(0))),
        ],
        vec![function.clone()],
        function,
        vec![],
        vec![],
        vec![],
        vec![],
    );
    let encoded = wire(&file);
    assert_eq!(projection(file.view()), projection(encoded.view()));
    for (effect, value) in file.setup().sources() {
        assert_eq!(encoded.view().setup().sources().get(effect), Some(*value));
    }
    assert_eq!(encoded.view().setup().sources().get(&Effect(8)), None);
}

#[test]
fn byte_backed_callback_queries_follow_foreign_rebinding() {
    for rebind in [false, true] {
        let library = parse(
            &format!(
                "export var callback = () => 1; {}",
                if rebind {
                    "var callback = () => opaque();"
                } else {
                    ""
                }
            ),
            "",
        );
        let callback = library
            .symbols()
            .iter()
            .find(|s| s.name() == "callback")
            .unwrap()
            .id();
        let entry = parse(
            "import { callback } from 'library'; function read() { return callback(); } query(read);",
            "global fn query(callback) { call_effects = call(callback) returns void }",
        );
        let library = wire(&library);
        let entry = wire(&entry);
        let weak = Arc::downgrade(&library);
        let mut analysis = Analysis::new();
        analysis.add_file("library", library).unwrap();
        analysis.add_file("entry", entry).unwrap();
        let result = analysis
            .solve(&|_: &&str, reference: ExternalReferenceView<'_>| {
                Ok(match reference.binding() {
                    BindingView::Module { .. } => Resolution::Symbol(Symbol {
                        file: "library",
                        local: callback,
                    }),
                    BindingView::Global(path) if path.iter().eq(["query"]) => {
                        Resolution::Declaration(reference.binding().to_owned())
                    }
                    _ => Resolution::Unknown,
                })
            })
            .unwrap();
        assert_eq!(
            result
                .evaluation
                .effect(result.files["entry"].roots[1].effects),
            rebind
        );
        assert!(
            weak.upgrade().is_some(),
            "runtime templates must retain the byte owner"
        );
        drop(result);
        assert!(weak.upgrade().is_none());
    }
}

fn anchored(bytes: &[u8], text: &str) {
    let start = bytes.as_ptr() as usize;
    let pointer = text.as_ptr() as usize;
    assert!(
        pointer >= start && pointer + text.len() <= start + bytes.len(),
        "{text:?} is not borrowed"
    );
}
fn binding_strings(bytes: &[u8], binding: BindingView<'_>) {
    match binding {
        BindingView::Local(v) => anchored(bytes, v),
        BindingView::Global(path) => {
            for v in path {
                anchored(bytes, v);
            }
        }
        BindingView::Module { module, path } => {
            anchored(bytes, module);
            for v in path {
                anchored(bytes, v);
            }
        }
    }
}
fn expression_strings(bytes: &[u8], expression: HazardExprView<'_>) {
    match expression {
        HazardExprView::Read { value, .. } => anchored(bytes, value),
        HazardExprView::Or(children) => {
            for v in children {
                expression_strings(bytes, v);
            }
        }
        HazardExprView::Call {
            function,
            arguments,
            receiver,
        } => {
            anchored(bytes, function);
            for v in arguments {
                match v {
                    CallArgumentView::Value(v) | CallArgumentView::Spread(v) => anchored(bytes, v),
                    CallArgumentView::Forward => {}
                }
            }
            if let Some(v) = receiver {
                anchored(bytes, v);
            }
        }
        _ => {}
    }
}

#[test]
fn nested_contract_strings_borrow_bytes_after_extraction_is_dropped() {
    let file = parse(
        "import { run } from 'package'; function local(x) { return x; } local({}); run();",
        r#"
        global symbol magic { read_effects = true }
        fn local(x) { call_effects = false returns = x }
        module "package" {
            export fn run(callback, value, ...rest) {
                call_effects = value.property_read_effects | call(callback, value, ...rest, this = value)
                updates value.call_effects |= unknown()
                returns void
            }
        }
        global fn make(callback, ...) { call_effects = call(callback, ...) returns.*_effects = false }
        "#,
    );
    let expected = projection(file.view());
    let owner = wire(&file);
    drop(file);
    assert_eq!(projection(owner.view()), expected);
    let bytes = owner.bytes();
    for symbol in owner.view().symbols() {
        anchored(bytes, symbol.name());
    }
    for reference in owner.view().external_references() {
        binding_strings(bytes, reference.binding());
    }
    for matched in owner.view().definition_matches() {
        binding_strings(bytes, matched.binding());
    }
    for symbol in owner.view().declarations().global_symbols() {
        anchored(bytes, symbol.name());
    }
    for definition in owner.view().declarations().definitions() {
        binding_strings(bytes, definition.binding());
        for name in definition.parameters() {
            anchored(bytes, name);
        }
        if let Some(rest) = definition.rest() {
            anchored(bytes, rest);
        }
        if let ReturnsView::Argument(v) = definition.returns() {
            anchored(bytes, v);
        }
        for fields in [definition.hazards(), definition.return_hazards()] {
            for (_, expr) in fields.fields() {
                expression_strings(bytes, expr);
            }
            if let Some(expr) = fields.default_expression() {
                expression_strings(bytes, expr);
            }
        }
        for update in definition.updates() {
            anchored(bytes, update.target());
            expression_strings(bytes, update.expression());
        }
    }
    let pointer = owner.view().symbols().get(0).unwrap().name().as_ptr() as usize;
    let clone = owner.clone();
    std::thread::spawn(move || {
        assert_eq!(
            clone.view().symbols().get(0).unwrap().name().as_ptr() as usize,
            pointer
        );
        clone.view().validate().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn wire_semantic_validation_rejects_the_same_invalid_precursors() {
    let file = parse("function read(x) { return x.key; } read({});", "");
    for (field, replacement) in [
        ("version", json!(999)),
        ("values", json!([])),
        (
            "symbols",
            json!([{"id": 99, "name": "read", "span": [9, 13], "value": 2}]),
        ),
    ] {
        let mut object = serde_json::to_value(&*file).unwrap();
        object[field] = replacement;
        let malformed: ParsedFile = serde_json::from_value(object).unwrap();
        let wire = wire(&malformed);
        assert!(malformed.validate().is_err());
        assert!(wire.view().validate().is_err());
        assert!(Analysis::new().add_file((), wire).is_err());
    }
}

#[test]
fn encoder_embeds_typed_ecco_in_the_module_builder() {
    let file = parse("const text = 'nested'; text;", "");
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let ecco = file.encode_flatbuffer(&mut builder);
    let top_level_await = builder.create_vector::<w::Span>(&[]);
    let effects = builder.create_vector::<flatbuffers::WIPOffset<w::EffectSite<'_>>>(&[]);
    let invocations = builder.create_vector::<flatbuffers::WIPOffset<w::Invocation<'_>>>(&[]);
    let emission = w::EmissionFactsT::default().pack(&mut builder);
    let bindings = builder.create_vector::<flatbuffers::WIPOffset<w::Binding<'_>>>(&[]);
    let references = builder.create_vector::<flatbuffers::WIPOffset<w::Reference<'_>>>(&[]);
    let dependencies = builder.create_vector::<flatbuffers::WIPOffset<w::Dependency<'_>>>(&[]);
    let imports = builder.create_vector::<flatbuffers::WIPOffset<w::ImportBinding<'_>>>(&[]);
    let exports = builder.create_vector::<flatbuffers::WIPOffset<w::Export<'_>>>(&[]);
    let directives = builder.create_vector::<flatbuffers::WIPOffset<w::Directive<'_>>>(&[]);
    let fingerprint = w::Fingerprint::default();
    let facts = w::ModuleFacts::create(
        &mut builder,
        &w::ModuleFactsArgs {
            ecco: Some(ecco),
            ecco_declarations_hash: Some(&fingerprint),
            top_level_await: Some(top_level_await),
            effects: Some(effects),
            invocations: Some(invocations),
            emission: Some(emission),
            bindings: Some(bindings),
            references: Some(references),
            dependencies: Some(dependencies),
            imports: Some(imports),
            exports: Some(exports),
            directives: Some(directives),
            ..Default::default()
        },
    );
    builder.finish(facts, None);
    let bytes = builder.finished_data();
    let module = flatbuffers::root::<w::ModuleFacts<'_>>(bytes).unwrap();
    let view: ParsedFileView<'_> = module.ecco().unwrap().into();
    view.validate().unwrap();
    assert_eq!(projection(file.view()), projection(view));
    anchored(bytes, view.symbols().get(0).unwrap().name());
}
