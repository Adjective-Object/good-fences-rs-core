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
    .into_owned()
    .unwrap()
}

#[test]
fn lazy_options_jsx_values_omit_only_optional_evidence_in_owned_and_wire_facts() {
    // JSX extraction is supported below the public bundler's transform-policy gate.
    for value in [
        "<Label/>",
        "<><Label>{capture}</Label></>",
        "makeLabel({nested: <Label/>})",
    ] {
        let source = format!(
            "const capture = 1;
             const mod = new Unrelated(() => import('./child.mjs'),
                 {{name: {value}, capture}});
             const selected = new LI(mod, ns => ns.first);
             const other = import('./other.mjs');"
        );
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            &source,
            std::path::Path::new("lazy.tsx"),
            crate::ModuleFormat::Esm,
        )
        .unwrap();
        let semantic = oxc_semantic::SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        assert!(semantic.errors.is_empty());
        let owned = crate::owned::extract(&parsed.program, &semantic.semantic).unwrap();
        let bytes = owned.encode_module(crate::ModuleBody::Esm, &[]);
        let module =
            flatbuffers::root::<cache_flatbuffers::wire::CachedModule<'_>>(&bytes).unwrap();
        let wire = crate::views::ModuleFacts::Wire(module.facts());
        let owned = crate::views::ModuleFacts::from(&owned);
        assert_eq!(wire.emission(), owned.emission());
        assert_eq!(wire.references(), owned.references());
        assert_eq!(wire.dependencies(), owned.dependencies());
        for facts in [owned, wire] {
            let modules = facts.emission().lazy_modules();
            assert_eq!(modules.len(), 1, "{value}");
            let module = modules.get(0).unwrap();
            assert!(module.options().is_some());
            assert!(module.options_properties().is_none(), "{}", value);
            assert_eq!(module.arguments().len(), 2);
            assert_eq!(facts.emission().lazy_selectors().len(), 1);
            assert_eq!(facts.dependencies().len(), 2);
            assert!(facts
                .references()
                .iter()
                .any(|r| r.name() == "Label" && r.read() && !r.type_only()));
        }
    }
}

#[test]
fn lazy_options_record_complete_plain_property_shapes_not_effect_assumptions() {
    for (options, properties) in [
        ("{}", Some(0)),
        (
            "{name: effect(capture), capture, 'other': () => capture,}",
            Some(3),
        ),
        ("{initializer: undefined}", None),
        ("{...capture}", None),
        ("{[key]: capture}", None),
        ("{get name() { return capture; }}", None),
        ("{__proto__: capture}", None),
        ("unknown", None),
        ("effect()", None),
        ("{name: /[)]/}", None),
        ("{name: capture / 2}", None),
        ("{name: capture /= 2}", None),
        ("{name: 'path/to/bundle'}", Some(1)),
    ] {
        let source = format!("const mod = new M(() => import('./target'), {options});");
        let facts = parse(&source);
        let candidate = &facts.emission().lazy_modules()[0];
        assert_eq!(
            candidate.options_properties().map(<[_]>::len),
            properties,
            "{options}"
        );
        if let Some(properties) = candidate.options_properties() {
            assert!(properties.iter().all(|p| contains(
                candidate.options().unwrap().span(),
                p.key()
            ) && contains(
                candidate.options().unwrap().span(),
                p.value()
            )));
        }
    }
}

#[test]
fn lazy_candidates_preserve_aliases_binding_identity_and_original_slots() {
    let source =
        "import { LazyModule as LM, LazyImport as LI, LazyAction as LA } from './runtime.mjs';
        export const mod = new (LM)((() => (import('./child.mjs'))), options()),
            first = new LI((mod), ((ns) => (ns.first)), extra()),
            second = new LA(mod, ns => ns.second, ...rest);
        function shadow(LM, LazyImport) {
            const mod = new LM(() => import('./other.mjs'), { initializer: initialize });
            return new LazyImport(mod, ns => ns.first);
        }
        const Unknown = LI;
        const unknown = new Unknown(mod, ns => ns.first);
        const unsupported = new LI(mod, ns => ns.first());
        const computed = new LI(mod, ns => ns['first']);
        const block = new LI(mod, ns => { return ns.first; });
        const optional = new LI(mod, ns => ns?.first);
        const destructured = new LI(mod, ({ first }) => first);
        const asyncGetter = new LI(mod, async ns => ns.first);
        const notCallback = new LM(async () => import('./child.mjs'));
        const parameterCallback = new LM(x => import('./child.mjs'));
        const patternCallback = new LM(() => import('./parts/' + name));";
    let facts = parse(source);
    let emission = facts.emission();
    assert_eq!(emission.lazy_modules().len(), 2);
    assert_eq!(emission.lazy_selectors().len(), 4);
    let modules = emission.lazy_modules();
    assert_ne!(modules[0].binding(), modules[1].binding());
    let constructors: Vec<_> = modules
        .iter()
        .map(|c| facts.references()[c.constructor() as usize].binding())
        .collect();
    assert_ne!(constructors[0], constructors[1]);
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    assert_eq!(
        text(modules[0].callback().span()),
        "(() => (import('./child.mjs')))"
    );
    assert_eq!(text(modules[0].options().unwrap().span()), "options()");
    assert_eq!(
        text(modules[1].options().unwrap().span()),
        "{ initializer: initialize }"
    );
    let selectors = emission.lazy_selectors();
    assert_eq!(
        selectors
            .iter()
            .map(|c| (c.kind(), c.property()))
            .collect::<Vec<_>>(),
        [
            (LazyWrapperKind::LazyImport, "first"),
            (LazyWrapperKind::LazyAction, "second"),
            (LazyWrapperKind::LazyImport, "first"),
            (LazyWrapperKind::Unknown, "first")
        ]
    );
    assert_eq!(selectors[0].module_binding(), modules[0].binding());
    assert_eq!(selectors[1].module_binding(), modules[0].binding());
    assert_eq!(selectors[2].module_binding(), modules[1].binding());
    assert_eq!(text(selectors[0].module_argument().span()), "(mod)");
    assert_eq!(text(selectors[0].getter().span()), "((ns) => (ns.first))");
    assert_eq!(text(selectors[0].arguments()[2].span()), "extra()");
    assert_eq!(text(selectors[1].arguments()[2].span()), "...rest");
    assert!(!modules[0].options().unwrap().operations().is_empty());
    assert_ne!(modules[0].segment(), selectors[0].segment());
    // Unsupported uses remain ordinary references for the later all-reference gate.
    assert_eq!(
        facts
            .references()
            .iter()
            .filter(|r| r.binding() == Some(modules[0].binding()))
            .count(),
        9
    );
    for c in selectors {
        assert!(contains(
            c.getter().span(),
            facts.references()[c.getter_reference() as usize].span()
        ));
    }
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
fn dynamic_namespace_observations_check_every_alias_reference_and_receiver() {
    for (body, expected) in [
        (
            "return (await import('./x')).value;",
            Some(vec![("value".into(), false)]),
        ),
        (
            "const ns = await import('./x'); ns.change(); return ns.value;",
            Some(vec![("change".into(), true), ("value".into(), false)]),
        ),
        (
            "return import('./x').then(ns => ns.value);",
            Some(vec![("value".into(), false)]),
        ),
        (
            "return ((import('./x'))).then(((ns) => (ns).value));",
            Some(vec![("value".into(), false)]),
        ),
        (
            "return import('./x').then(ns => { function other(ns) { return ns.shadow; } return ns.value; });",
            Some(vec![("value".into(), false)]),
        ),
        (
            "return import('./x').then(ns => { ns.method(); return ns.method; });",
            Some(vec![("method".into(), false), ("method".into(), true)]),
        ),
        (
            "return import('./x').then(ns => () => ns.value);",
            Some(vec![("value".into(), false)]),
        ),
        (
            "return ((await (import('./x')))).method();",
            Some(vec![("method".into(), true)]),
        ),
        ("return await import('./x').value;", None),
        ("return import('./x').then(ns => ns);", None),
        ("return import('./x').then(ns => Object.keys(ns));", None),
        ("return import('./x').then(ns => ns[key]);", None),
        ("return import('./x').then(ns => { ns = other; return ns.value; });", None),
        ("return import('./x').then(ns => { var ns = other; return ns.value; });", None),
        ("return import('./x').then(ns => { function ns() {} return ns.value; });", None),
        ("return import('./x').then(ns => ns.value = 1);", None),
        ("return import('./x').then(ns => ns.value++);", None),
        ("return import('./x').then(ns => delete ns.value);", None),
        ("return import('./x').then(ns => ns`tag`);", None),
        ("return import('./x').then(ns => ({...ns}));", None),
        ("return import('./x').then(ns => 'value' in ns);", None),
        ("return () => import('./x');", None),
        ("return async () => await import('./x');", None),
        (
            "const {value} = await import('./x'); return value;",
            Some(vec![("value".into(), false)]),
        ),
        ("await import('./x');", Some(vec![])),
        ("return import('./x');", None),
        ("const ns = await import('./x'); return ns;", None),
        (
            "const ns = await import('./x'); return Object.keys(ns);",
            None,
        ),
        ("const ns = await import('./x'); return ns['value'];", None),
        ("const ns = await import('./x'); ns.value++;", None),
        ("const ns = await import('./x'); delete ns.value;", None),
        ("const ns = await import('./x'); ns.value = 1;", None),
        ("let ns = await import('./x'); return ns.value;", None),
        (
            "const {value = 1} = await import('./x'); return value;",
            None,
        ),
        ("const {...rest} = await import('./x'); return rest;", None),
    ] {
        let facts = parse(&format!("export async function load() {{ {body} }}"));
        assert_eq!(facts.emission().dynamic_namespaces().len(), 1);
        assert_eq!(
            facts.emission().dynamic_namespaces()[0].properties(),
            expected.as_deref(),
            "{body}"
        );
    }
    let facts = parse("const ns = await import('./x'); export {ns}; ns.value;");
    assert_eq!(facts.emission().dynamic_namespaces()[0].properties(), None);
    assert!(!facts.emission().uses_this());
    assert!(parse("export function receiver() { return this; }")
        .emission()
        .uses_this());
}

#[test]
fn dynamic_namespace_observations_preserve_occurrences_and_property_pairs() {
    let source = "export async function load() {
        (await import('./x')).first;
        return import('./x').then(ns => { ns.second; ns.first; ns.second; ns.second(); });
    }";
    let facts = parse(source);
    let observations = facts.emission().dynamic_namespaces();
    assert_eq!(observations.len(), 2);
    assert_ne!(observations[0].dependency(), observations[1].dependency());
    assert_eq!(
        observations
            .iter()
            .map(|o| o.dependency())
            .collect::<Vec<_>>(),
        facts
            .dependencies()
            .iter()
            .map(|d| d.id())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        observations[0].properties(),
        Some([("first".into(), false)].as_slice())
    );
    assert_eq!(
        observations[1].properties(),
        Some(
            [
                ("first".into(), false),
                ("second".into(), false),
                ("second".into(), true)
            ]
            .as_slice()
        )
    );
}

#[test]
fn deferred_forms_keep_split_initializer_slots_in_original_coordinates() {
    let source = "export const secret = makeSecret(), other = {secret}; function read() { return secret; } trace(); class Unsupported {} const {x} = {};";
    let facts = parse(source);
    let recipes = &facts.emission().segments();
    for (index, name, text) in [(0, "secret", "makeSecret()"), (1, "other", "{secret}")] {
        let DeferredForm::Variable {
            binding,
            initializer: Some(slot),
        } = &recipes[index].deferred()
        else {
            panic!("missing split variable alternative");
        };
        assert_eq!(
            facts
                .bindings()
                .iter()
                .find(|b| b.id() == *binding)
                .unwrap()
                .name(),
            name
        );
        assert_eq!(
            &source[slot.span().start as usize..slot.span().end as usize],
            text
        );
        assert!(!slot.operations().is_empty());
        assert!(slot.operations().iter().all(|&i| contains(
            slot.span(),
            recipes[index].operations()[i as usize]
                .kind()
                .span(&facts, &recipes[index])
        )));
    }
    assert_eq!(*recipes[2].deferred(), DeferredForm::Function);
    assert_eq!(*recipes[3].deferred(), DeferredForm::Effect);
    assert_eq!(*recipes[4].deferred(), DeferredForm::Unsupported);
    assert_eq!(*recipes[5].deferred(), DeferredForm::Unsupported);
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
    let recipe = parsed.facts().emission().segments().get(0).unwrap();
    assert!(!recipe.connector_only());
    assert!(recipe.placement().declarations().is_empty());
    assert_eq!(recipe.operations().len(), 3);
    assert!(recipe.operations().iter().all(|o| matches!(
        o.kind(),
        crate::views::OperationKind::Dependency { .. }
            | crate::views::OperationKind::SourceHint { .. }
    )));
}
