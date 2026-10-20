use super::*;
use ast_segmenter::{parse_module, ModuleFormat};
use source_graph::OwnedSourceFileInput;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn input(path: &str, source: &str) -> OwnedSourceFileInput {
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        ModuleFormat::Esm,
    )
    .unwrap();
    let resolutions = parsed
        .facts
        .dependencies
        .iter()
        .map(|d| (d.id, DependencyTarget::File(d.specifier.clone().into())))
        .collect();
    OwnedSourceFileInput::from_parsed(path.into(), parsed, resolutions)
}

fn retained_sources<'a>(
    graph: &SourceGraph,
    plan: &RetentionPlan,
    sources: &'a [&str],
) -> Vec<(u32, &'a str, Vec<RetentionReason>)> {
    plan.segments
        .iter()
        .map(|s| {
            let span = graph.file_segments(s.key.file_id).unwrap()[s.key.segment_idx as usize].span;
            (
                s.key.file_id,
                &sources[s.key.file_id as usize][span.start as usize..span.end as usize],
                s.reasons.clone(),
            )
        })
        .collect()
}

#[test]
fn runtime_imports_evaluate_unreferenced_exports_and_no_export_modules() {
    let sources = [
        "import {unused} from 'lib.ts'; import type {T} from 'types.ts';",
        "import 'effects.ts'; export const unused = initialize(helper); const helper = 1; function dead() { nested(); }",
        "first(); second();",
        "export type T = string; typeOnlyEffect();",
    ];
    let graph = SourceGraph::from_owned([
        input("entry.ts", sources[0]),
        input("lib.ts", sources[1]),
        input("effects.ts", sources[2]),
        input("types.ts", sources[3]),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0, 1, 2]);
    insta::assert_debug_snapshot!(retained_sources(&graph, &plan, &sources));
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 2)));
    assert!(!plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 3)));
    assert!(plan
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::EffectOrder
            && e.from == GraphNode::Segment(SegmentKey::new(2, 1))
            && e.to == GraphNode::Segment(SegmentKey::new(2, 0))));
    assert_eq!(plan, retain(&graph, &[0], &[]).unwrap());
}

#[test]
fn effect_retention_crosses_pure_import_and_reexport_connectors() {
    let graph = SourceGraph::from_owned([
        input("entry.mjs", "import 'pure.mjs'; const needed = 1;"),
        input(
            "pure.mjs",
            "export {value} from 'barrel.mjs'; const dead = 2;",
        ),
        input("barrel.mjs", "export {value} from 'effects.mjs';"),
        input("effects.mjs", "export const value = 3; run();"),
    ]);
    let root = SegmentKey::new(0, 1);
    let effect = SegmentKey::new(3, 1);
    let plan = retain(&graph, &[], &[GraphNode::Segment(root)]).unwrap();
    assert_eq!(plan.modules, [0, 1, 2, 3]);
    assert_eq!(plan.segments.len(), 2);
    assert_eq!(plan.effect_order, [effect]);
    assert!(plan.edges.iter().any(|edge| {
        edge.from == GraphNode::Module(effect.file_id)
            && edge.to == GraphNode::Segment(effect)
            && edge.kind == RetentionEdgeKind::ModuleEffect
    }));
    let reached = chunks::reference_reachability(
        &graph,
        &plan,
        &policy::EffectPolicy::default(),
        &[chunks::RootSeed {
            id: chunks::RootId::Entry("binding".into()),
            evaluation_roots: vec![],
            binding_roots: vec![GraphNode::Segment(root)],
        }],
    )
    .unwrap();
    assert!(reached
        .values()
        .next()
        .unwrap()
        .contains(&chunks::node_id(&graph, GraphNode::Segment(effect)).unwrap()));
    assert!(!plan.edges.iter().any(|edge| {
        edge.from == GraphNode::Segment(root)
            && matches!(
                edge.kind,
                RetentionEdgeKind::Binding | RetentionEdgeKind::EffectOrder
            )
    }));
}

#[test]
fn local_effect_retention_does_not_pin_pure_binding_order() {
    let graph = SourceGraph::from_owned([input(
        "entry.mjs",
        "const needed = 1; first(); const dead = 2; second();",
    )]);
    let pure = SegmentKey::new(0, 0);
    let first = SegmentKey::new(0, 1);
    let second = SegmentKey::new(0, 3);
    let plan = retain(&graph, &[], &[GraphNode::Segment(pure)]).unwrap();
    assert_eq!(plan.effect_order, [first, second]);
    for effect in [first, second] {
        assert!(plan.edges.iter().any(|edge| {
            edge.from == GraphNode::Module(pure.file_id)
                && edge.to == GraphNode::Segment(effect)
                && edge.kind == RetentionEdgeKind::ModuleEffect
        }));
    }
    assert!(!plan
        .segments
        .iter()
        .any(|segment| segment.key == SegmentKey::new(0, 2)));
    assert!(!plan.edges.iter().any(|edge| {
        edge.kind == RetentionEdgeKind::EffectOrder
            && (edge.from == GraphNode::Segment(pure) || edge.to == GraphNode::Segment(pure))
    }));
    assert!(plan.edges.iter().any(|edge| {
        edge.kind == RetentionEdgeKind::EffectOrder
            && edge.from == GraphNode::Segment(second)
            && edge.to == GraphNode::Segment(first)
    }));
}

#[test]
fn sibling_static_imports_finish_in_source_order_through_pure_barrels() {
    for barrel in [false, true] {
        for reverse in [false, true] {
            let mut inputs = vec![
                input("entry.mjs", "import 'z.mjs'; import 'a.mjs'; entry();"),
                input(
                    "z.mjs",
                    if barrel {
                        "export * from 'z-leaf.mjs';"
                    } else {
                        "z();"
                    },
                ),
                input(
                    "a.mjs",
                    if barrel {
                        "import 'a-leaf.mjs';"
                    } else {
                        "a();"
                    },
                ),
                input("z-leaf.mjs", "zLeaf();"),
                input("a-leaf.mjs", "aLeaf();"),
            ];
            if reverse {
                inputs.reverse();
            }
            let graph = SourceGraph::from_owned(inputs);
            let entry = graph.file_id(Path::new("entry.mjs")).unwrap();
            let plan = retain(&graph, &[entry], &[]).unwrap();
            let order: Vec<_> = plan
                .effect_order
                .iter()
                .map(|key| graph.file_path(key.file_id).unwrap().to_str().unwrap())
                .collect();
            assert_eq!(
                order,
                if barrel {
                    vec!["z-leaf.mjs", "a-leaf.mjs", "entry.mjs"]
                } else {
                    vec!["z.mjs", "a.mjs", "entry.mjs"]
                },
                "barrel={barrel}, reverse={reverse}"
            );
            assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        }
    }
}

#[test]
fn async_static_subtrees_do_not_serialize_later_siblings() {
    for transitive in [false, true] {
        let graph = SourceGraph::from_owned([
            input(
                "entry.mjs",
                "import 'a.mjs'; import 'b.mjs'; import 'c.mjs'; entry();",
            ),
            input(
                "a.mjs",
                if transitive {
                    "import 'child.mjs'; after();"
                } else {
                    "before(); await pending; after();"
                },
            ),
            input("b.mjs", "b();"),
            input("c.mjs", "c();"),
            input("child.mjs", "before(); await pending; after();"),
        ]);
        let plan = retain(&graph, &[0], &[]).unwrap();
        let dependencies: HashMap<_, Vec<_>> = plan
            .edges
            .iter()
            .filter(|edge| edge.kind == RetentionEdgeKind::EffectOrder)
            .fold(HashMap::new(), |mut map, edge| {
                map.entry(edge.from).or_default().push(edge.to);
                map
            });
        let depends_on = |from: SegmentKey, to: SegmentKey| {
            let mut pending = vec![GraphNode::Segment(from)];
            let mut seen = HashSet::new();
            while let Some(node) = pending.pop() {
                if !seen.insert(node) {
                    continue;
                }
                if node == GraphNode::Segment(to) {
                    return true;
                }
                pending.extend(dependencies.get(&node).into_iter().flatten().copied());
            }
            false
        };
        let b = SegmentKey::new(2, 0);
        let c = SegmentKey::new(3, 0);
        let after = SegmentKey::new(1, if transitive { 1 } else { 2 });
        assert!(!depends_on(b, after), "transitive={transitive}");
        assert!(depends_on(c, b), "later synchronous siblings stay ordered");
        assert!(depends_on(SegmentKey::new(0, 3), after));
        assert!(depends_on(SegmentKey::new(0, 3), c));
        if transitive {
            assert!(!depends_on(b, SegmentKey::new(4, 2)));
            assert!(depends_on(after, SegmentKey::new(4, 2)));
        } else {
            assert!(depends_on(after, SegmentKey::new(1, 0)));
        }
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }
}

#[test]
fn static_import_order_does_not_reenter_shared_dependencies() {
    for entry in [
        "import 'z.mjs'; import 'a.mjs'; entry();",
        "import 'z.mjs'; import 'z.mjs'; import 'a.mjs'; entry();",
    ] {
        let graph = SourceGraph::from_owned([
            input("entry.mjs", entry),
            input("z.mjs", "import 'a.mjs'; z();"),
            input("a.mjs", "a();"),
        ]);
        let plan = retain(&graph, &[0], &[]).unwrap();
        let order: Vec<_> = plan.effect_order.iter().map(|key| key.file_id).collect();
        assert_eq!(order, [2, 1, 0]);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }
}

#[test]
fn late_static_imports_and_reexports_are_hoisted_without_false_cycles() {
    for declaration in ["import 'dep.mjs';", "export * from 'dep.mjs';"] {
        let graph = SourceGraph::from_owned([
            input("entry.mjs", &format!("before(); {declaration} after();")),
            input("dep.mjs", "dep();"),
        ]);
        let plan = retain(&graph, &[0], &[]).unwrap();
        assert_eq!(
            plan.effect_order,
            [
                SegmentKey::new(1, 0),
                SegmentKey::new(0, 0),
                SegmentKey::new(0, 2)
            ],
            "{declaration}"
        );
        assert!(
            plan.warnings.is_empty(),
            "{declaration}: {:?}",
            plan.warnings
        );
    }
}

#[test]
fn effect_retention_and_order_edges_scale_linearly() {
    let source = "run();".repeat(1000);
    let graph = SourceGraph::from_owned([input("entry.mjs", &source)]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.effect_order.len(), 1000);
    assert!(plan.edges.len() <= 3000, "{} edges", plan.edges.len());
    let reached = chunks::reference_reachability(
        &graph,
        &plan,
        &policy::EffectPolicy::default(),
        &[chunks::RootSeed {
            id: chunks::RootId::Entry("main".into()),
            evaluation_roots: vec![0],
            binding_roots: vec![],
        }],
    )
    .unwrap();
    assert_eq!(reached.values().next().unwrap().len(), 1001);
}

#[test]
fn effect_order_projects_source_order_and_static_imports_through_pure_modules() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.mjs",
            "import 'pure.mjs'; first(); const dead = 1; second();",
        ),
        input("pure.mjs", "import 'leaf.mjs'; const dead = 2;"),
        input("leaf.mjs", "leafFirst(); const dead = 3; leafSecond();"),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(
        plan.effect_order,
        [
            SegmentKey::new(2, 0),
            SegmentKey::new(2, 2),
            SegmentKey::new(0, 1),
            SegmentKey::new(0, 3),
        ]
    );
    assert!(plan.warnings.is_empty());
    let root = chunks::RootSeed {
        id: chunks::RootId::Entry("main".into()),
        evaluation_roots: vec![0],
        binding_roots: vec![],
    };
    let mut no_order = plan.clone();
    no_order
        .edges
        .retain(|edge| edge.kind != RetentionEdgeKind::EffectOrder);
    assert_eq!(
        chunks::reference_reachability(
            &graph,
            &plan,
            &policy::EffectPolicy::default(),
            std::slice::from_ref(&root)
        )
        .unwrap(),
        chunks::reference_reachability(
            &graph,
            &no_order,
            &policy::EffectPolicy::default(),
            &[root]
        )
        .unwrap(),
    );
}

#[test]
fn effect_order_cycles_warn_with_canonical_members_and_order() {
    let files = [
        ("a.mjs", "import 'pure.mjs'; a();"),
        ("pure.mjs", "import 'b.mjs';"),
        ("b.mjs", "import 'a.mjs'; b();"),
    ];
    let analyze = |reverse: bool| {
        let mut inputs: Vec<_> = files
            .iter()
            .map(|(path, source)| input(path, source))
            .collect();
        if reverse {
            inputs.reverse();
        }
        let graph = SourceGraph::from_owned(inputs);
        let plan = retain(&graph, &[graph.file_id(Path::new("a.mjs")).unwrap()], &[]).unwrap();
        for edge in plan
            .edges
            .iter()
            .filter(|edge| edge.kind == RetentionEdgeKind::EffectOrder)
        {
            let position = |node| {
                plan.effect_order
                    .iter()
                    .position(|&key| GraphNode::Segment(key) == node)
                    .unwrap()
            };
            assert!(
                position(edge.to) < position(edge.from),
                "cycle constraints must be resolved"
            );
        }
        let order: Vec<_> = plan
            .effect_order
            .iter()
            .map(|key| {
                (
                    graph.file_path(key.file_id).unwrap().to_owned(),
                    key.segment_idx,
                )
            })
            .collect();
        (order, plan.warnings)
    };
    let (order, warnings) = analyze(false);
    assert_eq!(order, [("a.mjs".into(), 1), ("b.mjs".into(), 1)]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("a.mjs:"));
    assert!(warnings[0].contains(" -> b.mjs:"));
    assert_eq!((order, warnings), analyze(true));

    let pure_cycle = SourceGraph::from_owned([
        input("a.mjs", "import 'b.mjs';"),
        input("b.mjs", "import 'a.mjs';"),
    ]);
    assert!(retain(&pure_cycle, &[0], &[]).unwrap().warnings.is_empty());
}

#[test]
fn effect_order_does_not_cross_async_or_deferred_activation_boundaries() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.mjs",
            "export function later() { return import('async.mjs'); } now();",
        ),
        input("async.mjs", "asyncEffect();"),
        input(
            "deferred.cjs",
            "exports.later = () => require('required.cjs');",
        ),
        input("required.cjs", "requiredEffect();"),
    ]);
    let plan = retain(
        &graph,
        &[0, 2],
        &[GraphNode::Segment(SegmentKey::new(0, 0))],
    )
    .unwrap();
    assert!(!plan.modules.contains(&1));
    assert_eq!(plan.async_boundaries.len(), 1);
    assert!(
        plan.modules.contains(&3),
        "deferred code is available, not eagerly scheduled"
    );
    assert!(!plan.edges.iter().any(|edge| {
        edge.kind == RetentionEdgeKind::EffectOrder
            && matches!((edge.from, edge.to),
                (GraphNode::Segment(from), GraphNode::Segment(to))
                    if from.file_id != to.file_id)
    }));
}

#[test]
fn effect_projection_recomputes_after_policy_and_source_changes() {
    let mut graph = SourceGraph::from_owned([input(
        "entry.mjs",
        "function f() {} /* @__PURE__ */ f(); second();",
    )]);
    let policy = policy::EffectPolicy::default();
    let initial = retain_with_policy(&graph, &[0], &[], &policy).unwrap();
    assert_eq!(initial.effect_order, [SegmentKey::new(0, 2)]);
    let strict = policy.clone().with_pure_annotations(false);
    assert!(!initial.is_current(&graph, &strict));
    let changed_policy = retain_with_policy(&graph, &[0], &[], &strict).unwrap();
    assert_eq!(
        changed_policy.effect_order,
        [SegmentKey::new(0, 1), SegmentKey::new(0, 2)]
    );
    assert!(changed_policy
        .edges
        .iter()
        .any(|edge| edge.kind == RetentionEdgeKind::EffectOrder));
    graph
        .patch_owned(
            Path::new("entry.mjs"),
            input("entry.mjs", "const nowPure = 1;"),
        )
        .unwrap();
    assert!(!initial.is_current(&graph, &policy));
    assert!(!changed_policy.is_current(&graph, &strict));
    let changed_source = retain_with_policy(&graph, &[0], &[], &strict).unwrap();
    assert!(changed_source.effect_order.is_empty());
    assert!(changed_source.edges.is_empty());
}

#[test]
fn binding_demand_keeps_checked_barrel_routes_without_rooting_every_binding() {
    let sources = [
        "import {used} from 'barrel.ts'; consume(used);",
        "export {used} from 'leaf.ts';",
        "export const used = 1; export const dead = 2; initialize();",
    ];
    let graph = SourceGraph::from_owned([
        input("entry.ts", sources[0]),
        input("barrel.ts", sources[1]),
        input("leaf.ts", sources[2]),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(2, 0)));
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 0)));
    assert!(!plan.segments.iter().any(|s| s.key == SegmentKey::new(2, 1)));
    let export = graph
        .resolve_export(1, "used", source_graph::Namespace::Value)
        .unwrap();
    let direct = retain(&graph, &[], &export.route).unwrap();
    assert_eq!(direct.modules, [1, 2]);
}

#[test]
fn cycles_and_empty_modules_preserve_identity_and_source_order() {
    let graph = SourceGraph::from_owned([
        input(
            "a.ts",
            "import 'b.ts'; import 'empty.ts'; first(); second();",
        ),
        input("b.ts", "import 'a.ts'; third();"),
        input("empty.ts", ""),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0, 1, 2]);
    assert_eq!(plan.segments.len(), 3);
    assert_eq!(
        plan.edges
            .iter()
            .filter(|e| e.kind == RetentionEdgeKind::StaticEvaluation)
            .count(),
        3
    );
    insta::assert_debug_snapshot!(plan);
}

#[test]
fn deferred_bodies_are_not_eager_effects_and_async_sites_are_boundaries() {
    let graph = SourceGraph::from_owned([
        input("entry.cjs", "function later() { return require('dep.cjs'); }"),
        input("dep.cjs", "run();"),
        input("esm.ts", "export function later() { return import('async.ts'); }"),
        input("async.ts", "asyncEffect();"),
        input("pure.ts", "function unused(x = unknown()) { hidden(); } const arrow = () => hidden(); class C { x = hidden(); }"),
    ]);
    let pure = retain(&graph, &[4], &[]).unwrap();
    assert!(pure.segments.is_empty());
    let exported = retain(&graph, &[], &[GraphNode::Segment(SegmentKey::new(2, 0))]).unwrap();
    assert_eq!(exported.modules, [2]);
    assert_eq!(exported.async_boundaries.len(), 1);
    let cjs = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(cjs.modules, [0, 1]);
    let require = cjs
        .edges
        .iter()
        .find(|e| e.kind == RetentionEdgeKind::Require)
        .unwrap();
    assert!(graph.edges()[require.source_edge.unwrap()]
        .enclosing_function
        .is_some());
    assert!(!cjs
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::StaticEvaluation));
}

#[test]
fn rejects_invalid_roots_legacy_inputs_and_link_errors() {
    let graph = SourceGraph::from_owned([input("empty.ts", "")]);
    assert!(retain(&graph, &[42], &[]).is_err());
    assert!(retain(&graph, &[], &[GraphNode::Segment(SegmentKey::new(0, 0))]).is_err());
    let legacy = SourceGraph::new(std::iter::once(source_graph::SourceFileInput {
        source_file_path: "legacy.ts".into(),
        segments: vec![],
        resolved_reexport_paths: Default::default(),
        resolved_import_paths: Default::default(),
    }));
    assert!(retain(&legacy, &[0], &[]).is_err());
    let broken = SourceGraph::from_owned([
        input("a.ts", "import {missing} from 'b.ts';"),
        input("b.ts", ""),
    ]);
    assert!(retain(&broken, &[0], &[]).is_err());
}

#[test]
fn namespace_demand_retains_exports_not_private_declarations_and_plans_expire_on_mutation() {
    let mut graph = SourceGraph::from_owned([
        input("entry.ts", "import * as ns from 'lib.ts'; consume(ns);"),
        input("lib.ts", "export const one = 1; export const two = 2; const privateValue = () => import('unused.ts');"),
        input("unused.ts", "export const dead = 3;"),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(
        plan.segments.iter().filter(|s| s.key.file_id == 1).count(),
        2
    );
    assert!(plan.async_boundaries.is_empty());
    assert_eq!(plan.modules, [0, 1]);
    assert!(plan.is_current(&graph, &policy::EffectPolicy::default()));
    graph
        .patch_owned(
            Path::new("lib.ts"),
            input("lib.ts", "export const one = 3;"),
        )
        .unwrap();
    assert!(!plan.is_current(&graph, &policy::EffectPolicy::default()));
}

#[test]
fn cache_ranges_and_facts_produce_the_same_plan_without_tracker_maps() {
    let sources = [
        ("entry.ts", "import {value} from 'lib.ts'; consume(value);"),
        ("lib.ts", "export const value = 1; initialize();"),
    ];
    let cold = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
    let cached = SourceGraph::from_owned(sources.iter().map(|(path, source)| {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new(path),
            ModuleFormat::Esm,
        )
        .unwrap();
        let record = segment_cache::CacheRecord::from_parsed(source, &parsed).unwrap();
        let bytes = record.encode(source).unwrap();
        drop(parsed);
        let record = segment_cache::CacheRecord::decode(&bytes, source, record.config).unwrap();
        let resolutions = record
            .module
            .facts
            .dependencies
            .iter()
            .map(|d| (d.id, DependencyTarget::File(d.specifier.clone().into())))
            .collect();
        OwnedSourceFileInput::from_parsed(
            (*path).into(),
            record.to_parsed(source).unwrap(),
            resolutions,
        )
    }));
    assert_eq!(
        retain(&cold, &[0], &[]).unwrap(),
        retain(&cached, &[0], &[]).unwrap()
    );
}

#[test]
fn external_evaluation_is_explicit_and_type_reexports_do_not_evaluate() {
    let mut entry = input(
        "entry.ts",
        "import 'external'; export type {T} from 'types.ts';",
    );
    let dependency = entry.facts.dependencies[0].id;
    entry
        .resolutions
        .insert(dependency, DependencyTarget::External("external".into()));
    let graph =
        SourceGraph::from_owned([entry, input("types.ts", "export type T = string; run();")]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0]);
    assert_eq!(plan.edges.len(), 1);
    assert!(matches!(plan.edges[0].to, GraphNode::Unlinked { .. }));
}
