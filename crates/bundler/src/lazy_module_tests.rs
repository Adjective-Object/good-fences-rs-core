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

fn planned(input: &BTreeMap<PathBuf, String>, wire: bool) -> (SourceGraph, ChunkPlan) {
    let graph = graph_with_storage(input, wire, ModuleFormat::Esm, wire);
    let plan = crate::chunks::partition_with_options(
        &graph,
        &roots(&graph),
        &EffectPolicy::default(),
        &OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        },
    )
    .unwrap();
    (graph, plan)
}

fn segment(
    graph: &SourceGraph,
    input: &BTreeMap<PathBuf, String>,
    path: &str,
    text: &str,
) -> source_graph::SegmentKey {
    let file = graph.file_id(Path::new(path)).unwrap();
    let index = graph
        .file_segments(file)
        .unwrap()
        .iter()
        .position(|segment| {
            input[Path::new(path)][segment.span.start as usize..segment.span.end as usize]
                .contains(text)
        })
        .unwrap();
    source_graph::SegmentKey::new(file, index as u32)
}

fn mask<'a>(
    graph: &SourceGraph,
    plan: &'a ChunkPlan,
    key: source_graph::SegmentKey,
) -> &'a BTreeSet<RootId> {
    &plan.node_roots[&crate::chunks::node_id(graph, GraphNode::Segment(key)).unwrap()]
}

#[test]
fn lazy_overlays_replace_broad_demand_with_independent_selector_roots() {
    let mut input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);
        export const b = new R(mod, ns => ns.second);
        const unused = new A(mod, ns => ns.unused);",
    );
    input.insert(
        "target.mjs".into(),
        "import './effects.mjs'; export * from './leaf.mjs';".into(),
    );
    input.insert(
        "leaf.mjs".into(),
        "function shared() { return 1; }
        export function first() { return shared(); }
        export function second() { return shared() + 1; }
        export function unused() { return import('./dead.mjs'); }"
            .into(),
    );
    input.insert("dead.mjs".into(), "throw new Error('dead');".into());
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert!(mask(&graph, &plan, module.owner).is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const unused")
        )
        .is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function unused")
        )
        .is_empty());
        assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
        assert!(!plan.retention.edges.iter().any(|edge| {
            edge.source_edge == Some(module.source_edge)
                || edge.to == GraphNode::Segment(module.owner)
        }));
        let a = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first"),
        );
        let b = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function second"),
        );
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert!(a.is_disjoint(b));
        let union: BTreeSet<_> = a.union(b).cloned().collect();
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "leaf.mjs", "function shared")
            ),
            union
        );
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "effects.mjs", "barrelEffect")
            ),
            union
        );
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "runtime.mjs", "class ModuleCtor")
        )
        .is_empty());
        let manifest = diagnostic_manifest(&graph, &plan, &EffectPolicy::default()).unwrap();
        for (site, activation) in &plan.async_activations {
            let crate::chunks::AsyncOrigin::Lazy {
                declaration,
                module_candidate,
                selector,
            } = activation.origin
            else {
                panic!("no original import activation");
            };
            assert_eq!(declaration, module.owner);
            assert_eq!(module_candidate, module.candidate);
            let selected = module
                .selectors
                .iter()
                .find(|s| s.candidate == selector)
                .unwrap();
            assert_eq!(site.offset, selected.construction.start);
            let facade = &manifest.async_facades[site];
            assert_eq!(facade.target, NodeId::Module("target.mjs".into()));
            assert_eq!(
                facade.consumer,
                crate::chunks::node_id(&graph, GraphNode::Segment(selected.owner)).unwrap()
            );
            assert_eq!(facade.demand_roots.len(), 1);
        }
    }
}

#[test]
fn lazy_residual_options_and_unused_wrapper_arguments_preserve_real_dependencies() {
    let input = input("const capture = {value: 7};
        function helper() { return capture; }
        function record(value) { globalThis.seen = value; return value; }
        const mod = new M(() => import('./target.mjs'), {name: record(capture), other: () => helper()});
        export const a = new R(mod, ns => ns.first, () => helper());
        export const b = new R(mod, ns => ns.first);
        const unused = new A(mod, ns => ns.second, record(capture));
        const dead = new R(mod, ns => ns.second);");
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert!(!plan
            .retention
            .edges
            .iter()
            .any(|edge| edge.kind == crate::RetentionEdgeKind::Binding
                && edge.to == GraphNode::Segment(module.owner)));
        assert!(plan
            .retention
            .edges
            .iter()
            .any(|edge| edge.kind == crate::RetentionEdgeKind::PreservedSlot
                && edge.to == GraphNode::Segment(module.owner)));
        let original = plan
            .retention
            .segments
            .iter()
            .find(|s| s.key == module.owner)
            .unwrap();
        assert!(original
            .reasons
            .iter()
            .all(|reason| !reason.demands_value()));
        let symbols = crate::symbols::Symbols::link(&graph, &plan).unwrap();
        assert!(!symbols
            .definitions
            .contains_key(&(module.owner.file_id, module.binding)));
        assert!(!plan
            .retention
            .execution
            .accesses
            .iter()
            .any(|access| access.declaration == module.owner && access.segment != module.owner));
        assert!(!plan
            .retention
            .execution
            .constraints
            .iter()
            .any(|constraint| constraint.before == module.owner
                && module
                    .selectors
                    .iter()
                    .any(|selector| selector.owner == constraint.after)
                && matches!(
                    constraint.kind,
                    crate::execution_dependencies::ConstraintKind::Initialization
                        | crate::execution_dependencies::ConstraintKind::ReadWrite
                )));
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == module.owner)
            .unwrap()
            .summary;
        assert_eq!(
            summary
                .rewrites
                .iter()
                .filter(|r| r.invocation == module.construction)
                .count(),
            1
        );
        assert!(!summary
            .sites
            .iter()
            .any(|site| site.span() == module.construction));
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
        for text in [
            "const capture",
            "function helper",
            "function record",
            "const unused",
        ] {
            assert!(
                !mask(&graph, &plan, segment(&graph, &input, "entry.mjs", text)).is_empty(),
                "{text}"
            );
        }
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const dead")
        )
        .is_empty());
        let unused = segment(&graph, &input, "entry.mjs", "const unused");
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == unused)
            .unwrap()
            .summary;
        assert!(summary.is_effectful());
        assert_eq!(summary.rewrites.len(), 1);
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
    }
}

#[test]
fn lazy_pure_options_capture_is_retained_at_original_slot_without_module_value() {
    let input = input(
        "const captured = {};
        const mod = new M(() => import('./target.mjs'), {other: () => captured});
        export const a = new R(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        let retained = plan
            .retention
            .segments
            .iter()
            .find(|s| s.key == module.owner)
            .unwrap();
        assert_eq!(retained.reasons, [crate::RetentionReason::PreservedSlot]);
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const captured")
        )
        .is_empty());
    }
}

#[test]
fn lazy_selected_demands_discover_nested_instances_but_not_dead_ones() {
    let mut input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);",
    );
    input.insert(
        "leaf.mjs".into(),
        "import {next} from './nested.mjs';
        export function first() { return next; } export function second() { return 2; }"
            .into(),
    );
    input.insert(
        "nested.mjs".into(),
        format!(
            "{IMPORTS}
        const nested = new M(() => import('./last.mjs'));
        export const next = new R(nested, ns => ns.value);
        const dead = new M(() => import('./dead.mjs'));
        const ignored = new R(dead, ns => ns.value);"
        ),
    );
    input.insert(
        "last.mjs".into(),
        "export const value = 3; export const sibling = 4;".into(),
    );
    input.insert("dead.mjs".into(), "export const value = 5;".into());
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.lazy_modules.modules().len(), 3);
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert_eq!(
            mask(
                &graph,
                &plan,
                segment(&graph, &input, "last.mjs", "const value")
            )
            .len(),
            1
        );
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "last.mjs", "const sibling")
        )
        .is_empty());
        assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
    }
}

#[test]
fn lazy_required_wrapper_values_activate_without_statement_binding_demand() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        globalThis.used = new R(mod, ns => ns.first);
        new A(mod, ns => ns.second);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.async_activations.len(), 1);
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first")
        )
        .is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function second")
        )
        .is_empty());
        assert!(mask(&graph, &plan, segment(&graph, &input, "entry.mjs", "new A")).is_empty());
    }
}

#[test]
fn lazy_invocation_overlay_preserves_other_expressions_in_the_same_statement() {
    let input = input(
        "function effect() { globalThis.seen = true; }
        const mod = new M(() => import('./target.mjs'));
        const unused = (new A(mod, ns => ns.second), effect());
        export const a = new R(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        let selector = &module.selectors[0];
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == selector.owner)
            .unwrap()
            .summary;
        assert!(summary.is_effectful());
        assert!(summary
            .rewrites
            .iter()
            .any(|r| r.invocation == selector.construction));
        assert!(!summary
            .sites
            .iter()
            .any(|s| s.span() == selector.construction));
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "function effect")
        )
        .is_empty());
        assert!(plan.retention.async_boundaries.is_empty());
        assert_eq!(plan.async_activations.len(), 1);
    }
}

#[test]
fn lazy_same_export_selectors_keep_distinct_activation_and_root_identities() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);
        export const b = new A(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.async_activations.len(), 2);
        let selected = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first"),
        );
        assert_eq!(selected.len(), 2);
        assert!(selected
            .iter()
            .all(|root| matches!(root, RootId::AsyncExport { name, .. } if name == "first")));
        assert!(plan
            .async_activations
            .values()
            .all(|activation| activation.roots.len() == 1));
    }
}

#[test]
fn lazy_mixed_instance_retains_the_original_broad_graph_unchanged() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first); globalThis.escape = mod;",
    );
    for wire in [false, true] {
        let (graph, enabled) = planned(&input, wire);
        let disabled =
            crate::chunks::partition(&graph, &roots(&graph), &EffectPolicy::default()).unwrap();
        assert!(enabled.lazy_modules.modules().is_empty());
        assert_eq!(enabled.retention, disabled.retention);
        assert_eq!(enabled.roots, disabled.roots);
        assert_eq!(enabled.async_activations, disabled.async_activations);
        assert!(!mask(
            &graph,
            &enabled,
            segment(&graph, &input, "leaf.mjs", "function second")
        )
        .is_empty());
    }
}

#[test]
fn lazy_certification_uses_canonical_aliased_barrel_identities_for_overlays() {
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
        assert_ne!(plan.retention, disabled.retention);
        assert_ne!(plan.roots, disabled.roots);
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
        esm(&graph, &disabled, &policy, &input).unwrap();
        let error = esm_with_options(&graph, &plan, &policy, &input, &options).unwrap_err();
        assert!(error.to_string().contains("post-placement lazy wiring"));
        assert!(esm(&graph, &plan, &policy, &input).is_err());
        assert!(deferred_esm(&graph, &plan, &policy, &input).is_err());
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
            let plan = crate::chunks::partition_with_options(
                &graph,
                &roots(&graph),
                &EffectPolicy::default(),
                &OutputOptions {
                    lazy_runtimes: vec![runtime()],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(plan.retention.async_boundaries.len(), 1, "{extra}");
            assert!(
                !plan
                    .retention
                    .async_boundaries
                    .contains(&checked.modules()[0].source_edge),
                "{extra}"
            );
            assert_eq!(plan.async_activations.len(), 2, "{extra}");
            assert_eq!(
                plan.async_activations
                    .values()
                    .filter(|activation| activation.origin == crate::chunks::AsyncOrigin::Import)
                    .count(),
                1,
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
