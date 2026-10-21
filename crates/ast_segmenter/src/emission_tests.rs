use super::*;

fn parse(source: &str) -> ModuleFacts {
    crate::parse_module(
        &logger::StdioLogger::new(),
        source,
        std::path::Path::new("input.mjs"),
        crate::ModuleFormat::Esm,
    )
    .unwrap()
    .into_facts()
}

#[test]
fn execution_contexts_keep_immediate_calls_defaults_and_instance_fields_distinct() {
    use crate::execution::ContextKind;
    let source = "let x = 0; const stored = (p = x++) => x++; (() => x++)(); class C { [x++] = x++; static value = x++; static { x++; } method(p = x++) { return () => x++; } }";
    let facts = parse(source);
    let execution = &facts.emission().execution();
    let accesses: Vec<_> = facts
        .references()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name() == "x")
        .map(|(index, _)| {
            let context = execution.references()[index];
            (
                execution.is_module_evaluation(context),
                context.map(|c| execution.contexts()[c as usize].kind()),
            )
        })
        .collect();
    assert_eq!(
        accesses,
        [
            (false, Some(ContextKind::Callable)),
            (false, Some(ContextKind::Callable)),
            (true, Some(ContextKind::Callable)),
            (true, None),
            (false, Some(ContextKind::InstanceInitializer)),
            (true, None),
            (true, None),
            (false, Some(ContextKind::Callable)),
            (false, Some(ContextKind::Callable)),
        ]
    );
    let nested = execution.contexts().last().unwrap();
    assert!(nested.parent().is_some());
    assert_eq!(
        facts
            .emission()
            .segments()
            .iter()
            .map(|r| r.placement().eager_writes().len())
            .collect::<Vec<_>>(),
        [0, 0, 1, 3]
    );
    assert_eq!(facts.emission().segments()[1].placement().writes().len(), 2);
}

#[test]
fn only_direct_synchronous_literal_calls_prove_module_execution() {
    let facts = parse("let x; function known(p = x) { return x; } known(); unknown(() => x); (function*(){ return x; })(); (async () => x)(); (() => () => x)(); (() => { (() => x)(); })();");
    let timing = &facts.emission().execution();
    let eager: Vec<_> = facts
        .references()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name() == "x")
        .map(|(i, _)| timing.is_module_evaluation(timing.references()[i]))
        .collect();
    assert_eq!(eager, [false, false, false, false, false, false, true]);
    assert!(facts.emission().segments()[1]
        .placement()
        .eager_reads()
        .is_empty());

    let facts = parse("let x; class C { [x](){ return x; } static [x] = x; field = () => x; }");
    let timing = &facts.emission().execution();
    let eager: Vec<_> = facts
        .references()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name() == "x")
        .map(|(i, _)| timing.is_module_evaluation(timing.references()[i]))
        .collect();
    assert_eq!(eager, [true, false, true, true, false]);
}

#[test]
fn execution_effects_use_the_owning_operation_context_not_callable_operands() {
    use crate::effects::EffectReason;
    for operation in [
        "class C extends async function() {} {}",
        "class C { [function() {}]() {} }",
        "class C { [() => 0] = 1; }",
        "const object = { [function() {}]: 1 };",
    ] {
        for (source, eager) in [
            (operation.to_owned(), true),
            (format!("(() => {{ {operation} }})();"), true),
            (
                format!("(() => {{ (() => {{ {operation} }})(); }})();"),
                true,
            ),
            (format!("function stored() {{ {operation} }}"), false),
            (
                format!("function stored() {{ (() => {{ {operation} }})(); }}"),
                false,
            ),
            (format!("(async () => {{ {operation} }})();"), false),
            (
                format!("class Outer {{ field = (() => {{ {operation} }})(); }}"),
                false,
            ),
        ] {
            let facts = parse(&source);
            let timing = &facts.emission().execution();
            let site = facts
                .effects()
                .iter()
                .find(|site| {
                    matches!(
                        site.reason(),
                        EffectReason::ClassHeritage | EffectReason::ComputedKey
                    )
                })
                .unwrap();
            assert_eq!(site.deferred().is_none(), source == operation, "{source}");
            assert_eq!(
                timing.effect_is_module_evaluation(site).unwrap(),
                eager,
                "{source}"
            );
            // Every operand here is a non-immediate literal, unlike its owning operation.
            assert!(
                !timing.is_module_evaluation(timing.context_at(site.span())),
                "{}",
                source
            );
        }
    }
    let facts = parse("function stored() { class C extends async function() {} {} }");
    let site = facts
        .effects()
        .iter()
        .find(|site| site.reason() == EffectReason::ClassHeritage)
        .unwrap();
    assert!(crate::execution::ExecutionFacts::default()
        .effect_is_module_evaluation(site)
        .is_err());
}

#[test]
fn connectors_and_placement_keep_initialization_and_writes_local() {
    let facts = parse("import {x} from './x'; export {}; ; before; let before = x; function f(){ return before; } before++; const arrow = () => before;");
    let recipes = &facts.emission().segments();
    assert_eq!(
        recipes
            .iter()
            .map(|r| r.connector_only())
            .collect::<Vec<_>>(),
        [true, true, true, false, false, false, false, false]
    );
    let before = facts
        .bindings()
        .iter()
        .find(|b| b.name() == "before")
        .unwrap()
        .id();
    assert_eq!(recipes[4].placement().forward_references(), [before]);
    assert_eq!(recipes[4].placement().declarations(), [before]);
    assert!(!recipes[4].placement().instantiable());
    assert!(recipes[5].placement().instantiable());
    assert_eq!(
        recipes[5].placement().declarations(),
        recipes[5].placement().hoisted()
    );
    assert!(recipes[5].placement().eager_reads().is_empty());
    assert!(recipes[7].placement().eager_reads().is_empty());
    assert_eq!(recipes[6].placement().writes().len(), 1);
    assert!(recipes[7].names().inference_sensitive());
    assert_eq!(
        recipes[5].names().named(),
        recipes[5].placement().declarations()
    );
}

#[test]
fn shorthand_expansion_distinguishes_literal_proto_from_patterns() {
    let facts = parse("let __proto__, x; const obj = {__proto__, x}; const {x: y, __proto__: z} = obj; ({__proto__, x = 1} = obj); function f({x = 2}) {}");
    let expansions: Vec<_> = facts
        .emission()
        .segments()
        .iter()
        .flat_map(|r| r.operations())
        .filter_map(|o| match o.kind() {
            OperationKind::Binding { shorthand, .. }
            | OperationKind::Reference { shorthand, .. }
                if *shorthand != Shorthand::None =>
            {
                Some(*shorthand)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        expansions,
        [
            Shorthand::Proto,
            Shorthand::Property,
            Shorthand::Property,
            Shorthand::Property,
            Shorthand::Property
        ]
    );
}

#[test]
fn namespaces_distinguish_values_from_receiver_tag_and_mutation() {
    let facts = parse("import * as ns from './ns'; ns.value; ns.method(); ns.tag`text`; ns.value = 1; ns.value++; delete ns.value; consume(ns); ns['computed']; consume(ns.value);");
    let uses: Vec<_> = facts
        .emission()
        .segments()
        .iter()
        .flat_map(|r| r.namespaces())
        .map(|u| u.access())
        .collect();
    assert!(matches!(uses[0], NamespaceAccess::DirectRead { property, .. } if property == "value"));
    assert_eq!(uses[1], &NamespaceAccess::Native(NamespaceReason::Receiver));
    assert_eq!(uses[2], &NamespaceAccess::Native(NamespaceReason::Tag));
    assert_eq!(uses[3], &NamespaceAccess::Native(NamespaceReason::Write));
    assert_eq!(uses[4], &NamespaceAccess::Native(NamespaceReason::Write));
    assert_eq!(uses[5], &NamespaceAccess::Native(NamespaceReason::Delete));
    assert_eq!(uses[6], &NamespaceAccess::Native(NamespaceReason::Other));
    assert_eq!(uses[7], &NamespaceAccess::Native(NamespaceReason::Other));
    assert!(matches!(uses[8], NamespaceAccess::DirectRead { .. }));
    let recipe = &facts.emission().segments()[1];
    assert!(matches!(
        recipe.operations()[0].kind(),
        OperationKind::Namespace { .. }
    ));
    assert_eq!(recipe.operations()[1].parent(), Some(0));
}

#[test]
fn named_and_default_imports_can_resolve_to_namespace_reexports() {
    for declaration in [
        "import {nested as alias} from './barrel';",
        "import alias from './barrel';",
    ] {
        let facts = parse(&format!(
            "{} alias.value; alias.method(); consume(alias); function shadow(alias){{ return alias.value; }}",
            declaration
        ));
        let uses: Vec<_> = facts
            .emission()
            .segments()
            .iter()
            .flat_map(|r| r.namespaces())
            .collect();
        assert_eq!(uses.len(), 3);
        assert!(
            matches!(&uses[0].access(), NamespaceAccess::DirectRead { property, .. } if property == "value")
        );
        assert_eq!(
            uses[1].access(),
            &NamespaceAccess::Native(NamespaceReason::Receiver)
        );
        assert_eq!(
            uses[2].access(),
            &NamespaceAccess::Native(NamespaceReason::Other)
        );
        assert!(facts
            .emission()
            .segments()
            .last()
            .unwrap()
            .namespaces()
            .is_empty());
    }
}

#[test]
fn export_wrappers_and_default_initialization_are_preclassified() {
    for (source, expected, inference) in [
        ("export default function() { return 1; }", "hoist", true),
        ("export default class {}", "initialize", true),
        ("export default () => 1", "initialize", true),
        ("export default function named() {}", "declaration", false),
        ("export const x = 1;", "declaration", false),
        ("export {};", "remove", false),
    ] {
        let facts = parse(source);
        let recipe = &facts.emission().segments()[0];
        let form = recipe
            .operations()
            .iter()
            .find_map(|o| match &o.kind() {
                OperationKind::Export { form, .. } => Some(form),
                _ => None,
            })
            .unwrap();
        let actual = match form {
            ExportForm::Default { hoisted: true, .. } => "hoist",
            ExportForm::Default { hoisted: false, .. } => "initialize",
            ExportForm::Declaration { .. } => "declaration",
            ExportForm::Remove => "remove",
        };
        assert_eq!(actual, expected, "{source}");
        assert_eq!(recipe.names().inference_sensitive(), inference, "{source}");
    }
}

fn lowering(facts: &ModuleFacts) -> &PureLowering {
    facts
        .emission()
        .segments()
        .iter()
        .flat_map(|r| r.operations())
        .find_map(|o| match &o.kind() {
            OperationKind::Invocation {
                invocation,
                lowering,
            } if facts.invocations()[*invocation as usize].pure().is_some() => Some(lowering),
            _ => None,
        })
        .unwrap()
}

#[test]
fn pure_chain_compilation_preserves_parenthesis_guards_and_nested_slots() {
    let facts = parse("/*#__PURE__*/ obj?.method?.(await value, ...items);");
    let plan = lowering(&facts);
    assert!(plan.crosses_await_guard());
    assert!(plan.guard_start().is_some());
    assert_eq!(plan.arguments().len(), 2);
    assert!(matches!(
        plan.chain().unwrap().steps()[0],
        ChainStep::Static { optional: true, .. }
    ));
    let facts = parse("/*#__PURE__*/ (obj?.method)(await value);");
    let plan = lowering(&facts);
    assert!(!plan.crosses_await_guard());
    assert!(plan.guard_start().is_none());
    assert!(plan.chain().unwrap().steps().is_empty());
    let facts = parse("async function f(){ /*#__PURE__*/ obj?.method(await value); }");
    assert!(lowering(&facts).crosses_await_guard());
    let facts = parse("/*#__PURE__*/ obj?.method(async () => await value);");
    assert!(!lowering(&facts).crosses_await_guard());
    let facts = parse("/*#__PURE__*/ obj.method?.()();");
    assert!(facts.emission().segments()[0].operations().iter().any(|o|
        matches!(&o.kind(), OperationKind::Invocation { lowering, .. } if lowering.chain().is_none())));
    let facts = parse("/*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(value));");
    let recipe = &facts.emission().segments()[0];
    let plan = lowering(&facts);
    assert!(!plan.arguments()[0].operations().is_empty());
    assert!(matches!(&plan.chain().unwrap().steps()[0],
        ChainStep::Computed { property, .. } if !property.operations().is_empty()));
    for (i, operation) in recipe.operations().iter().enumerate() {
        if let Some(parent) = operation.parent() {
            assert!((parent as usize) < i);
            assert!(contains(
                recipe.operations()[parent as usize]
                    .kind()
                    .span(&facts, recipe),
                operation.kind().span(&facts, recipe)
            ));
        }
    }
}

#[test]
fn hints_and_opaque_cjs_keep_only_source_local_request_slots() {
    let source = "const x = 1; /*# sourceURL=input.js */ use(x);";
    let facts = parse(source);
    assert_eq!(
        facts
            .emission()
            .segments()
            .iter()
            .flat_map(|r| r.operations())
            .filter(|o| matches!(o.kind(), OperationKind::SourceHint { .. }))
            .count(),
        0,
        "out-of-segment hint cleanup is deliberately out of scope"
    );
    let source = "const x = (/*# sourceURL=input.js */ 1);";
    let facts = parse(source);
    assert!(facts.emission().segments()[0]
        .operations()
        .iter()
        .any(|o| matches!(o.kind(), OperationKind::SourceHint { .. })));
    let source =
        "const x = require('./x'); function f(){ return import('./y'); } /*# sourceURL=input.js */";
    let parsed = crate::parse_module(
        &logger::StdioLogger::new(),
        source,
        std::path::Path::new("input.cjs"),
        crate::ModuleFormat::CommonJs,
    )
    .unwrap();
    let recipe = &parsed.facts().emission().segments()[0];
    assert!(!recipe.connector_only());
    assert!(recipe.placement().declarations().is_empty());
    assert_eq!(recipe.operations().len(), 3);
    assert!(recipe.operations().iter().all(|o| matches!(
        o.kind(),
        OperationKind::Dependency { .. } | OperationKind::SourceHint { .. }
    )));
}
