use super::*;
use crate::{parse_module, ModuleFormat};
use oxc_allocator::Allocator;
use std::path::Path;

fn parse(source: &str) -> ModuleFacts {
    let mut facts = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("test.ts"),
        ModuleFormat::Esm,
    )
    .unwrap()
    .into_facts()
    .into_owned()
    .unwrap();
    *facts.ecco_mut() = None;
    *facts.ecco_declarations_hash_mut() = Default::default();
    facts
}

#[test]
fn lone_surrogates_do_not_become_utf8_requests_or_known_properties() {
    for source in [
        r#"import "./\ud800";"#,
        r#"export { value } from "./\ud800";"#,
        r#"require("./\ud800");"#,
        r#"import("./" + "\ud800" + suffix);"#,
        r#"import(`./\ud800/${suffix}`);"#,
    ] {
        let error = parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new("surrogate.js"),
            ModuleFormat::Esm,
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("lone UTF-16 surrogate"),
            "{:?}",
            error
        );
    }
    let facts = parse(r#"object["\ud800"](); object["\ufffd"](); object["\ud83d\ude00"]();"#);
    let properties: Vec<_> = facts
        .invocations()
        .iter()
        .map(|invocation| match invocation.shape() {
            crate::invocation::CalleeShape::Member { property, .. } => property.as_deref(),
            other => panic!("expected member invocation, got {:?}", other),
        })
        .collect();
    assert_eq!(properties, [None, Some("\u{fffd}"), Some("\u{1f600}")]);
}

#[test]
fn async_deferred_tracks_the_executing_body_not_async_ancestry() {
    let facts = parse(
        r#"
module_sync;
function ordinary() {
    function_sync;
    async function nested() { nested_async; }
    restored_sync;
}
const arrow = async () => concise_async;
const syncArrow = () => concise_sync;
async function outer(
    value = parameter_sync,
    callback = async () => parameter_arrow_async,
    { [parameter_key_sync]: item = destructured_parameter_sync } = {}
) {
    before_async;
    (() => {
        callback_sync;
        (async () => callback_nested_async)();
        callback_restored_sync;
    })();
    after_callback_async;
    function inner(value = nested_parameter_sync) {
        inner_sync;
        async function again() { again_async; }
        inner_restored_sync;
    }
    const object = {
        [object_key_async]() { object_method_sync; },
        async [object_async_key_async](value = object_parameter_sync) { object_method_async; },
        get [getter_key_async]() { getter_sync; },
    };
    class C extends heritage_async {
        [field_key_async] = field_sync;
        static [static_field_key_async] = static_field_sync;
        callback = async () => field_arrow_async;
        static callback = async () => static_field_arrow_async;
        static {
            static_block_sync;
            (async () => static_block_arrow_async)();
            static_block_restored_sync;
        }
        [method_key_async](value = method_parameter_sync) { method_sync; }
        async [async_method_key_async](value = async_method_parameter_sync) { method_async; }
        async *generator() { generator_async; }
    }
    after_class_async;
}
class Outside {
    async [module_method_key_sync]() { module_method_async; }
}
after_module_sync;
"#,
    );
    let contexts: Vec<_> = facts
        .references()
        .iter()
        .map(|reference| {
            assert_eq!(
                reference.async_deferred(),
                reference.name().ends_with("_async"),
                "{}",
                reference.name()
            );
            (reference.name(), reference.async_deferred())
        })
        .collect();
    insta::assert_debug_snapshot!(contexts);
}

#[test]
fn lexical_identity_survives_forward_uses_shadowing_and_redeclarations() {
    let source = "later; hoisted; let later = 1; var hoisted; var hoisted;\n\
        function outer({value: later = hoisted}, ...rest) {\n\
          let [first, ...tail] = rest; { const later = first; use(later); }\n\
          return () => later + hoisted + tail.length;\n\
        }\n\
        try {} catch ({message}) { use(message); }\n\
        class C { method(arg) { return arg + later; } }";
    let facts = parse(source);
    let later: Vec<_> = facts
        .bindings()
        .iter()
        .filter(|b| b.name() == "later")
        .collect();
    assert_eq!(later.len(), 3);
    let outer_later = later.iter().find(|b| b.top_level()).unwrap();
    assert_eq!(facts.references()[0].binding(), Some(outer_later.id()));
    assert!(facts.references()[0].span().start < outer_later.id().0);
    let hoisted = facts
        .bindings()
        .iter()
        .find(|b| b.name() == "hoisted")
        .unwrap();
    assert_eq!(hoisted.declarations().len(), 2);
    assert!(hoisted.function_scoped());
    assert_eq!(facts.references()[1].binding(), Some(hoisted.id()));
    for name in ["first", "tail", "rest", "message", "arg"] {
        let binding = facts.bindings().iter().find(|b| b.name() == name).unwrap();
        assert!(!binding.top_level());
        assert!(facts
            .references()
            .iter()
            .any(|r| r.binding() == Some(binding.id())));
    }
    for binding in later {
        assert!(facts
            .references()
            .iter()
            .any(|r| r.binding() == Some(binding.id())));
    }
    for reference in facts.references() {
        assert_eq!(
            &source[reference.span().start as usize..reference.span().end as usize],
            reference.name()
        );
    }
    for binding in facts.bindings() {
        for span in binding.declarations() {
            assert_eq!(
                &source[span.start as usize..span.end as usize],
                binding.name()
            );
        }
    }
}

#[test]
fn occurrence_dto_snapshot_and_arena_reset() {
    let source = "#!/usr/bin/env node\n'use strict';\n\
        import main, {\"str name\" as alias, type Shape} from './same';\n\
        import * as ns from './same'; import './same';\n\
        alias; type T = Shape; const value: T = main;\n\
        export {alias as \"public name\", type T};\n\
        export {default as other} from './same'; export * as all from './same';\n\
        export const {x: renamed, ...rest} = ns;\n\
        export default function named() { return import('./same'); }\n\
        import('./same');";
    let mut arena = Allocator::default();
    let facts = {
        let parsed = oxc_utils_parse::parse_bundler_file(
            &arena,
            source,
            Path::new("test.ts"),
            ModuleFormat::Esm,
        )
        .unwrap();
        let semantic = oxc_semantic::SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        assert!(
            semantic.diagnostics.is_empty(),
            "{:?}",
            semantic.diagnostics
        );
        extract(&parsed.program, &semantic.semantic).unwrap()
    };
    arena.reset();
    let _reuse = oxc_utils_parse::parse_ts(&arena, "const unrelated = 0;");
    assert_eq!(facts, parse(source));
    assert_eq!(facts, parse(source));
    assert_eq!(facts.dependencies().len(), 7);
    assert_eq!(facts.imports().len(), 4);
    assert_eq!(
        facts
            .dependencies()
            .iter()
            .filter(|d| d.kind() == DependencyKind::DynamicImport)
            .count(),
        2
    );
    assert_eq!(
        facts
            .dependencies()
            .iter()
            .filter(|d| d.kind() == DependencyKind::Require)
            .count(),
        0
    );
    insta::assert_debug_snapshot!(facts);
    drop(arena);
    assert_eq!(facts.hashbang().unwrap().value(), "/usr/bin/env node");
}

#[test]
fn type_and_runtime_references_are_not_collapsed() {
    let facts = parse("class C {} type T = C; let value: C; value = new C(); type U = typeof value; export type {T};");
    let c = facts.bindings().iter().find(|b| b.name() == "C").unwrap();
    let refs: Vec<_> = facts
        .references()
        .iter()
        .filter(|r| r.binding() == Some(c.id()))
        .collect();
    assert_eq!(
        refs.iter().map(|r| r.type_only()).collect::<Vec<_>>(),
        [true, true, false]
    );
    assert!(facts
        .references()
        .iter()
        .any(|r| r.name() == "value" && r.write() && !r.type_only()));
    assert!(facts
        .references()
        .iter()
        .any(|r| r.name() == "value" && r.type_only()));
    assert!(facts.exports()[0].type_only());
    let facts = parse("type T = string; export {T}; export default T;");
    assert!(facts.exports().iter().all(|e| e.type_only()));
    let facts = parse("export class C {} export interface C {x: number}");
    assert!(!facts.bindings()[0].type_only());
    assert_eq!(
        facts
            .exports()
            .iter()
            .map(|e| e.type_only())
            .collect::<Vec<_>>(),
        [false, true]
    );
    let facts = parse("export declare const value: number;");
    assert!(facts.bindings()[0].ambient());
}

#[test]
fn default_forms_and_export_destructuring_have_exact_mappings() {
    for source in [
        "export default 42;",
        "export default function() {};",
        "export default class {};",
    ] {
        let facts = parse(source);
        assert!(matches!(
            facts.exports()[0].value(),
            ExportValue::Expression(_)
        ));
    }
    let facts = parse("export const [a, {x: b = 1}, ...rest] = input; export /*gap*/ default a;");
    assert_eq!(
        facts.exports().iter().map(|e| e.name()).collect::<Vec<_>>(),
        [Some("a"), Some("b"), Some("rest"), Some("default")]
    );
    assert_eq!(facts.exports()[3].name_span(), None);
    let facts = parse("var x; export var x; export default x;");
    assert_eq!(
        facts.exports()[0].value(),
        &ExportValue::Binding(facts.bindings()[0].id())
    );
    assert_eq!(facts.exports()[0].name().as_deref(), Some("x"));
}

#[test]
fn nonliteral_dependencies_are_errors_but_shadowed_require_is_ordinary_code() {
    for source in ["import(path);", "require(path);", "require();"] {
        assert!(parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new("test.js"),
            ModuleFormat::Esm
        )
        .is_err());
    }

    assert!(parse("function f(require) { return require(path); }")
        .dependencies()
        .is_empty());
}

#[test]
fn relative_contexts_preserve_owned_patterns_and_argument_ranges() {
    let source = "export const load = (name, suffix) => import(`./parts/${name}/view.${suffix}`);";
    let facts = parse(source);
    let dependency = &facts.dependencies()[0];
    assert_eq!(dependency.pattern().unwrap(), &["./parts/", "/view.", ""]);
    assert_eq!(
        &source
            [dependency.specifier_span().start as usize..dependency.specifier_span().end as usize],
        "`./parts/${name}/view.${suffix}`"
    );
    assert!(dependency.matches_specifier("./parts/nested/name/view.mjs"));
    assert!(!dependency.matches_specifier("./other/name/view.mjs"));
    assert!(!dependency.matches_specifier("./parts/view.mjs"));
    let facts = parse("export const load = name => import('./parts/' + name + '.mjs');");
    assert_eq!(
        facts.dependencies()[0].pattern().unwrap(),
        &["./parts/", ".mjs"]
    );
    assert!(
        parse("function f(require) { return require('./parts/' + name); }")
            .dependencies()
            .is_empty()
    );
}
