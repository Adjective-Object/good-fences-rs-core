use super::*;
use crate::{
    build::OutputOptions,
    lazy_modules::{certify, ConstructorExport, LazyRuntime},
};
use ast_segmenter::emission::LazyWrapperKind;

const IMPORTS: &str = "import {Module as M, Read as R, Action as A} from './runtime-barrel.mjs';";
const RUNTIME: &str = "export class ModuleCtor { constructor(load, options) { this.load = load; this.options = options; } }
    export class ImportCtor { constructor(module, get, shouldGovern) { this.module = module; this.get = get; this.shouldGovern = shouldGovern; } }
    export class ActionCtor { constructor(module, get, options) { this.module = module; this.get = get; this.options = options; } }";

#[test]
fn lazy_certification_accepts_registered_immutable_default_constructor_expressions() {
    let mut input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
    input.insert(
        "runtime.mjs".into(),
        RUNTIME.replace("export class ModuleCtor", "export default class"),
    );
    input.insert("runtime-barrel.mjs".into(), "export {default as Module, ImportCtor as Read, ActionCtor as Action} from './runtime.mjs';".into());
    let mut runtime = runtime();
    runtime.module.export = "default".into();
    for wire in [false, true] {
        let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
        let checked = certify(&graph, &EffectPolicy::default(), &[runtime.clone()]).unwrap();
        assert_eq!(checked.modules().len(), 1);
        assert!(matches!(
            checked.modules()[0].constructor,
            source_graph::ExportIdentity::Expression(_)
        ));
    }
}

fn runtime() -> LazyRuntime {
    let export = |name: &str| ConstructorExport {
        module: "runtime.mjs".into(),
        export: name.into(),
    };
    LazyRuntime {
        module: export("ModuleCtor"),
        import: export("ImportCtor"),
        action: export("ActionCtor"),
    }
}

fn input(body: &str) -> BTreeMap<PathBuf, String> {
    sources(&[
        ("entry.mjs", &format!("{IMPORTS}\n{body}")),
        ("runtime.mjs", RUNTIME),
        ("runtime-barrel.mjs", "export {ModuleCtor as Module, ImportCtor as Read, ActionCtor as Action} from './runtime.mjs';"),
        ("target.mjs", "import './effects.mjs'; export {first, second} from './leaf.mjs';"),
        ("leaf.mjs", "export function first() { return 1; } export function second() { return 2; } export function unused() { return 3; }"),
        ("effects.mjs", "globalThis.barrelEffect = true;"),
    ])
}

fn roots(graph: &SourceGraph) -> Vec<RootSeed> {
    vec![RootSeed {
        id: RootId::Entry("entry".into()),
        evaluation_roots: vec![graph.file_id(Path::new("entry.mjs")).unwrap()],
        binding_roots: vec![GraphNode::Module(
            graph.file_id(Path::new("entry.mjs")).unwrap(),
        )],
    }]
}

#[test]
fn lazy_certification_uses_canonical_aliased_barrel_identities_without_transforming() {
    let input = input("const ModuleAlias = M, ReadAlias = R;
        const captured = {value: 1};
        function effect(value) { globalThis.seen = value; return value; }
        const mod = new ModuleAlias(() => import('./target.mjs'), { name: effect(captured), captured });
        export const a = new ReadAlias(mod, ns => ns.first, effect(captured)),
            b = new A(mod, ns => ns.second, {bundleTime: effect(captured)});
        function shadow(M, R) { const mod = new M(() => import('./target.mjs')); return new R(mod, ns => ns.first); }");
    for wire in [false, true] {
        let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
        let policy = EffectPolicy::default();
        let disabled = crate::chunks::partition(&graph, &roots(&graph), &policy).unwrap();
        assert!(disabled.lazy_modules.modules().is_empty());
        let options = OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        };
        let parses = ast_segmenter::module_parse_count();
        let plan = crate::chunks::partition_with_options(&graph, &roots(&graph), &policy, &options)
            .unwrap();
        assert_eq!(ast_segmenter::module_parse_count(), parses);
        assert!(plan.is_current_with_options(&graph, &policy, &options));
        assert!(!plan.is_current_with_options(&graph, &policy, &OutputOptions::default()));
        assert_eq!(plan.retention, disabled.retention);
        assert_eq!(plan.roots, disabled.roots);
        assert_eq!(plan.chunks, disabled.chunks);
        assert_eq!(plan.async_activations, disabled.async_activations);
        let [module] = plan.lazy_modules.modules() else {
            panic!("one top-level module")
        };
        assert_eq!(
            module.target,
            graph.file_id(Path::new("target.mjs")).unwrap()
        );
        assert_eq!(
            graph.edges()[module.source_edge].dependency,
            Some(module.dependency)
        );
        assert_eq!(module.selectors.len(), 2);
        assert_eq!(module.selectors[0].kind, LazyWrapperKind::LazyImport);
        assert_eq!(module.selectors[1].kind, LazyWrapperKind::LazyAction);
        assert!(module.selectors.iter().all(|s| matches!(s.selected.identity,
            source_graph::ExportIdentity::Binding(file, _) if Some(file) == graph.file_id(Path::new("leaf.mjs")))));
        let preserved = module.options.as_ref().unwrap();
        assert!(!preserved.operations.is_empty());
        assert!(!preserved.effects.is_empty());
        assert_eq!(preserved.invocations.len(), 1);
        let facts = graph.file_facts(module.owner.file_id).unwrap();
        assert!(preserved.references.iter().any(|&r| facts
            .references()
            .get(r as usize)
            .unwrap()
            .name()
            == "captured"));
        assert_eq!(module.selectors[0].arguments.len(), 3);
        assert_eq!(module.selectors[0].arguments[2].invocations.len(), 1);
        assert_eq!(module.selectors[1].arguments[2].invocations.len(), 1);
        let original = esm(&graph, &disabled, &policy, &input).unwrap();
        let certified = esm_with_options(&graph, &plan, &policy, &input, &options).unwrap();
        assert_eq!(original.files, certified.files);
        assert_eq!(original.members, certified.members);
        assert!(
            esm_with_options(&graph, &plan, &policy, &input, &OutputOptions::default()).is_err()
        );
        let changed_policy = EffectPolicy::default().with_pure_annotations(false);
        assert!(!plan
            .lazy_modules
            .is_current(&graph, &changed_policy, &options.lazy_runtimes));
        let mut other_graph = graph_with_storage(&input, !wire, ModuleFormat::Esm, wire);
        assert!(other_graph.remove_file(Path::new("effects.mjs")));
        assert!(!plan
            .lazy_modules
            .is_current(&other_graph, &policy, &options.lazy_runtimes));
    }
}

#[test]
fn lazy_certification_falls_back_for_the_entire_mixed_or_escaped_instance() {
    for extra in [
        "export {mod};",
        "globalThis.escape = mod;",
        "mod.getValue();",
        "mod.preload();",
        "unknown(mod);",
        "const alias = mod; const b = new R(alias, ns => ns.second);",
        "function neverCalled() { unknown(mod); }",
        "const Other = class {}; const b = new Other(mod, ns => ns.second);",
        "const b = new R(mod, ns => ns.second());",
        "const b = new R(mod, ns => ns['second']);",
        "const b = new A(mod, ns => ns.second, ...unknown);",
    ] {
        let input = input(&format!(
            "const mod = new M(() => import('./target.mjs'));
            export const a = new R(mod, ns => ns.first); {extra}
            const supported = new M(() => import('./target.mjs'));
            export const independent = new R(supported, ns => ns.second);"
        ));
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            let checked = certify(&graph, &EffectPolicy::default(), &[runtime()]).unwrap();
            assert_eq!(checked.modules().len(), 1, "{extra}");
            let facts = graph
                .file_facts(checked.modules()[0].owner.file_id)
                .unwrap();
            assert_eq!(
                facts
                    .bindings()
                    .iter()
                    .find(|b| b.id() == checked.modules()[0].binding)
                    .unwrap()
                    .name(),
                "supported",
                "{extra}"
            );
        }
    }
}

#[test]
fn lazy_certification_requires_structurally_initializer_free_options() {
    for (options, expected) in [
        ("", true),
        (", {}", true),
        (", {name: record(captured), captured}", true),
        (
            ", {'name': record(captured), \"other\": () => captured}",
            true,
        ),
        (", {initializer: initialize}", false),
        (", {initializer: undefined}", false),
        (", {'initializer': initialize}", false),
        (", {init\\u0069alizer: initialize}", false),
        (", {'\\x69nitializer': initialize}", false),
        (", {get initializer() { return initialize; }}", false),
        (", {get name() { return captured; }}", false),
        (", {name() { return captured; }}", false),
        (", {...other}", false),
        (", {[key()]: captured}", false),
        (", {__proto__: other}", false),
        (", unknownOptions", false),
        (", options()", false),
        (", undefined", false),
        (", null", false),
        (", {}, extra()", false),
        (", ...other", false),
        (", {name: captured / 2}", false),
        (", {name: /x/}", false),
    ] {
        let input = input(&format!(
            "const captured = 1; function record(x) {{ return x; }}
            const mod = new M(() => import('./target.mjs'){options});
            export const a = new R(mod, ns => ns.first);"
        ));
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            let checked = certify(&graph, &EffectPolicy::default(), &[runtime()]).unwrap();
            assert_eq!(!checked.modules().is_empty(), expected, "{options}");
        }
    }
}

#[test]
fn lazy_certification_falls_back_for_unsupported_typescript_options_on_owned_and_wire() {
    for value in ["makeLabel<First, Second>()", "<[First, Second]>captured"] {
        let mut input = input(&format!(
            "const captured = 1;
             const mod = new M(() => import('./target.mjs'), {{name: {value}}});
             export const a = new R(mod, ns => ns.first);"
        ));
        let entry = input.remove(Path::new("entry.mjs")).unwrap();
        input.insert("entry.ts".into(), entry);
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            for runtimes in [vec![], vec![runtime()]] {
                let checked = certify(&graph, &EffectPolicy::default(), &runtimes).unwrap();
                assert!(checked.modules().is_empty(), "{value}");
            }
        }
    }
}

#[test]
fn lazy_certification_rejects_nonlocal_context_commonjs_and_external_star_targets() {
    for mode in ["external", "context", "commonjs", "external-star"] {
        let mut input = input(
            "const mod = new M(() => import('./target.mjs'));
                    export const a = new R(mod, ns => ns.first);",
        );
        if mode == "commonjs" {
            input.remove(Path::new("target.mjs"));
            input.insert("target.cjs".into(), "module.exports = {first: 1};".into());
            let entry = input.get_mut(Path::new("entry.mjs")).unwrap();
            *entry = entry.replace("'./target.mjs'", "'./target.cjs'");
        } else if mode == "context" {
            let entry = input.get_mut(Path::new("entry.mjs")).unwrap();
            *entry = entry.replace("'./target.mjs'", "'./parts/' + name");
        } else if mode == "external-star" {
            input.insert(
                "target.mjs".into(),
                "export * from 'external'; export {first} from './leaf.mjs';".into(),
            );
        }
        let inputs = input
            .iter()
            .map(|(path, text)| {
                let format = if mode == "commonjs" && path == Path::new("target.cjs") {
                    ModuleFormat::CommonJs
                } else {
                    ModuleFormat::Esm
                };
                let parsed = parse_module(&logger::StdioLogger::new(), text, path, format).unwrap();
                let resolutions = parsed
                    .facts()
                    .dependencies()
                    .iter()
                    .map(|d| {
                        let target = if d.specifier() == "external" {
                            DependencyTarget::External("external".into())
                        } else if d.specifier() == "./target.mjs" && mode == "external" {
                            DependencyTarget::External("target".into())
                        } else if d.kind() == ast_segmenter::owned::DependencyKind::DynamicImport
                            && mode == "context"
                        {
                            DependencyTarget::Context(BTreeMap::from([(
                                "./parts/target.mjs".into(),
                                "target.mjs".into(),
                            )]))
                        } else {
                            DependencyTarget::File(normalize(
                                &path.parent().unwrap().join(d.specifier()),
                            ))
                        };
                        (d.id(), target)
                    })
                    .collect();
                OwnedSourceFileInput::from_parsed(path.to_path_buf(), parsed, resolutions)
            })
            .collect::<Vec<_>>();
        let graph = SourceGraph::from_owned(inputs);
        assert!(
            certify(&graph, &EffectPolicy::default(), &[runtime()])
                .unwrap()
                .modules()
                .is_empty(),
            "{mode}"
        );
    }
}

#[test]
fn lazy_certification_rejects_homonyms_shadows_and_mutable_constructor_identities() {
    for (runtime_source, body) in [
        (RUNTIME.to_owned(), "class LazyModule {}; const mod = new LazyModule(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "class LazyImport {}; const mod = new M(() => import('./target.mjs')); export const a = new LazyImport(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "function shadow(M, R) { const mod = new M(() => import('./target.mjs')); return new R(mod, ns => ns.first); }"),
        (RUNTIME.replace("export class ModuleCtor", "export let ModuleCtor = class"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (format!("{RUNTIME} ModuleCtor = class {{}};"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (format!("{RUNTIME} ImportCtor = class {{}};"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "let Alias = M; const mod = new Alias(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "let mod = new M(() => import('./target.mjs')); mod = other; export const a = new R(mod, ns => ns.first);"),
    ] {
        let mut input = input(body);
        input.insert("runtime.mjs".into(), runtime_source);
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            assert!(certify(&graph, &EffectPolicy::default(), &[runtime()]).unwrap().modules().is_empty(), "{body}");
        }
    }
}

#[test]
fn lazy_certification_rejects_unsafe_target_surfaces_and_wrong_registrations() {
    for target in [
        "export const then = () => {}; export {first, second} from './leaf.mjs';",
        "export * as first from './leaf.mjs'; export {second} from './leaf.mjs';",
        "export {second} from './leaf.mjs';",
    ] {
        let mut input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
        input.insert("target.mjs".into(), target.into());
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            assert!(certify(&graph, &EffectPolicy::default(), &[runtime()])
                .unwrap()
                .modules()
                .is_empty());
        }
    }
    let input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
    let graph = graph(&input, false);
    let mut unregistered = runtime();
    unregistered.module.module = "unrelated.mjs".into();
    assert!(certify(&graph, &EffectPolicy::default(), &[unregistered])
        .unwrap()
        .modules()
        .is_empty());
    let mut conflicting = runtime();
    std::mem::swap(&mut conflicting.import, &mut conflicting.action);
    for registrations in [
        vec![runtime(), conflicting.clone()],
        vec![conflicting, runtime()],
    ] {
        assert!(certify(&graph, &EffectPolicy::default(), &registrations)
            .unwrap()
            .modules()
            .is_empty());
    }
}
