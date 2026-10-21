use super::*;
use crate::{chunks::partition_conservative as partition, tests::input};
use source_graph::{DependencyTarget, GraphNode, OwnedSourceFileInput};
use std::path::Path;

fn entry(graph: &SourceGraph, path: &str, id: &str) -> RootSeed {
    RootSeed {
        id: RootId::Entry(id.into()),
        evaluation_roots: vec![graph.file_id(Path::new(path)).unwrap()],
        binding_roots: vec![],
    }
}

#[test]
fn shared_cycles_and_nested_async_facades_have_canonical_loading_dependencies() {
    let sources = [
        ("a.ts", "import 'shared.ts'; import('nested.ts');"),
        ("b.ts", "import 'shared.ts';"),
        ("shared.ts", "import 'cycle.ts'; shared();"),
        ("cycle.ts", "import 'shared.ts'; cycle();"),
        ("nested.ts", "import 'shared.ts'; import('empty.ts');"),
        ("empty.ts", ""),
    ];
    let policy = EffectPolicy::default();
    let graph = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
    let roots = [entry(&graph, "a.ts", "a"), entry(&graph, "b.ts", "b")];
    let plan = partition(&graph, &roots, &policy).unwrap();
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert_eq!(manifest.entries.len(), 2);
    assert_eq!(manifest.async_facades.len(), 2);
    assert!(!manifest.chunk_dependencies.is_empty());
    assert_eq!(
        plan.chunk_for(&NodeId::Module("shared.ts".into())),
        plan.chunk_for(&NodeId::Module("cycle.ts".into()))
    );
    for dependency in &manifest.chunk_dependencies {
        assert_ne!(dependency.from, dependency.to);
        assert!(dependency.from.0.is_subset(&dependency.to.0));
        assert!(manifest.chunks.contains_key(&dependency.from));
        assert!(manifest.chunks.contains_key(&dependency.to));
    }
    let entry_facade = &manifest.entries["a"];
    assert_eq!(
        entry_facade.demand.evaluation_modules,
        BTreeSet::from(["a.ts".into()])
    );
    for async_facade in manifest.async_facades.values() {
        let consumer = plan.chunk_for(&async_facade.consumer).unwrap();
        let target = plan.chunk_for(&async_facade.target).unwrap();
        assert_ne!(consumer, target);
        assert!(!manifest.chunk_dependencies.contains(&ChunkDependency {
            from: consumer,
            to: target.clone()
        }));
        assert!(async_facade.facade.load_chunks.contains(&target));
        assert!(!entry_facade.load_chunks.contains(&target));
    }
    assert!(manifest.synthetic_nodes.iter().any(|node| matches!(node, SyntheticNode::ModuleIdentity { module, .. } if module == Path::new("empty.ts"))));
    insta::assert_debug_snapshot!(manifest);
    let reordered = SourceGraph::from_owned(sources.iter().rev().map(|(p, s)| input(p, s)));
    let other_roots = [
        entry(&reordered, "b.ts", "b"),
        entry(&reordered, "a.ts", "a"),
    ];
    let other = partition(&reordered, &other_roots, &policy).unwrap();
    assert_eq!(
        manifest,
        diagnostic_manifest(&reordered, &other, &policy).unwrap()
    );
}

#[test]
fn shared_chunk_module_records_do_not_turn_deferred_require_into_eager_evaluation() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.cjs",
            "require('eager.cjs'); exports.later = () => require('lazy.cjs');",
        ),
        input("eager.cjs", "eager();"),
        input("lazy.cjs", "lazy();"),
    ]);
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &[entry(&graph, "entry.cjs", "main")], &policy).unwrap();
    assert_eq!(plan.chunks.len(), 1);
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert_eq!(
        manifest.entries["main"].demand.evaluation_modules,
        BTreeSet::from(["entry.cjs".into()])
    );
    assert_eq!(
        manifest
            .synthetic_nodes
            .iter()
            .filter(|n| matches!(n, SyntheticNode::ModuleIdentity { .. }))
            .count(),
        3
    );
    assert!(manifest.chunk_dependencies.is_empty());
    assert!(plan.retention.edges.iter().any(|e| e
        .source_edge
        .is_some_and(|i| graph.edges()[i].enclosing_function.is_some())
        && e.to == GraphNode::Module(2)));
    insta::assert_debug_snapshot!(manifest);
}

#[test]
fn external_async_facades_are_explicit_and_never_local_evaluation_roots() {
    let mut source: OwnedSourceFileInput =
        input("entry.ts", "import('external'); import('external');");
    for dependency in source.facts.dependencies() {
        source.resolutions.insert(
            dependency.id(),
            DependencyTarget::External("external".into()),
        );
    }
    let graph = SourceGraph::from_owned([source]);
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &[entry(&graph, "entry.ts", "main")], &policy).unwrap();
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert_eq!(manifest.async_facades.len(), 2);
    assert_eq!(plan.retention.modules, [0]);
    for facade in manifest.async_facades.values() {
        assert!(matches!(facade.target, NodeId::External { .. }));
        assert!(facade.facade.demand.evaluation_modules.is_empty());
    }
    insta::assert_debug_snapshot!(manifest);
}

#[test]
fn stale_partition_and_manifest_require_recomputation_not_emission() {
    let mut graph = SourceGraph::from_owned([
        input("entry.ts", "import('empty.ts');"),
        input("empty.ts", ""),
    ]);
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &[entry(&graph, "entry.ts", "main")], &policy).unwrap();
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert!(manifest.is_current(&graph, &policy));
    let changed_policy = policy.clone().with_pure_annotations(false);
    assert!(!manifest.is_current(&graph, &changed_policy));
    assert!(diagnostic_manifest(&graph, &plan, &changed_policy).is_err());
    graph
        .patch_owned(Path::new("empty.ts"), input("empty.ts", "changed();"))
        .unwrap();
    assert!(!manifest.is_current(&graph, &policy));
    assert!(diagnostic_manifest(&graph, &plan, &policy).is_err());
    let fresh = partition(
        &graph,
        &[entry(&graph, "entry.ts", "main")],
        &changed_policy,
    )
    .unwrap();
    assert!(diagnostic_manifest(&graph, &fresh, &changed_policy)
        .unwrap()
        .is_current(&graph, &changed_policy));
}

#[test]
fn namespace_only_binding_code_is_not_pulled_into_evaluation_only_root() {
    let graph = SourceGraph::from_owned([
        input("namespace.ts", "import * as ns from 'lib.ts'; use(ns);"),
        input("evaluation.ts", "import 'lib.ts';"),
        input("lib.ts", "export const pure = 1;"),
    ]);
    let policy = EffectPolicy::default();
    let namespace = entry(&graph, "namespace.ts", "namespace");
    let evaluation = entry(&graph, "evaluation.ts", "evaluation");
    let plan = partition(&graph, &[namespace.clone(), evaluation.clone()], &policy).unwrap();
    let declaration = NodeId::Segment {
        module: "lib.ts".into(),
        index: 0,
    };
    assert_eq!(
        plan.node_roots[&declaration],
        BTreeSet::from([namespace.id])
    );
    assert!(plan.node_roots[&NodeId::Module("lib.ts".into())].contains(&evaluation.id));
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    let binding_chunk = plan.chunk_for(&declaration).unwrap();
    assert!(manifest.entries["namespace"]
        .load_chunks
        .contains(&binding_chunk));
    assert!(!manifest.entries["evaluation"]
        .load_chunks
        .contains(&binding_chunk));
}

#[test]
fn precise_manifest_projects_one_original_activation_onto_independent_roots() {
    let sources = [
        ("entry.ts", "const {a, b} = await import('barrel.ts'); use(a, b);"),
        ("barrel.ts", "import 'effects.ts'; export {a, b} from 'leaf.ts';"),
        ("effects.ts", "effect();"),
        ("leaf.ts", "function shared() { return 1; } export function a() { return shared(); } export function b() { return shared(); }"),
    ];
    let graph = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
    let policy = EffectPolicy::default();
    let plan =
        crate::chunks::partition_precise(&graph, &[entry(&graph, "entry.ts", "main")], &policy)
            .unwrap();
    let manifest = diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert_eq!(manifest.async_facades.len(), 1);
    let (site, activation) = manifest.async_facades.first_key_value().unwrap();
    let a = RootId::AsyncExport {
        site: site.clone(),
        name: "a".into(),
    };
    let b = RootId::AsyncExport {
        site: site.clone(),
        name: "b".into(),
    };
    let both = BTreeSet::from([a.clone(), b.clone()]);
    assert_eq!(activation.target, NodeId::Module("barrel.ts".into()));
    assert_eq!(
        activation
            .demand_roots
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        both
    );
    assert!(!plan.roots.contains_key(&RootId::Async(site.clone())));
    let a_node = NodeId::Segment {
        module: "leaf.ts".into(),
        index: 1,
    };
    let b_node = NodeId::Segment {
        module: "leaf.ts".into(),
        index: 2,
    };
    let effect_node = NodeId::Segment {
        module: "effects.ts".into(),
        index: 0,
    };
    let a_chunk = ChunkId(BTreeSet::from([a.clone()]));
    let b_chunk = ChunkId(BTreeSet::from([b.clone()]));
    let shared_chunk = ChunkId(both);
    assert_eq!(plan.chunk_for(&a_node), Some(a_chunk.clone()));
    assert_eq!(plan.chunk_for(&b_node), Some(b_chunk.clone()));
    assert_eq!(plan.chunk_for(&effect_node), Some(shared_chunk.clone()));
    assert_eq!(
        activation.demand_roots[&a].load_chunks,
        BTreeSet::from([a_chunk.clone(), shared_chunk.clone()])
    );
    assert_eq!(
        activation.demand_roots[&b].load_chunks,
        BTreeSet::from([b_chunk.clone(), shared_chunk.clone()])
    );
    assert_eq!(
        activation.facade.load_chunks,
        BTreeSet::from([a_chunk.clone(), b_chunk.clone(), shared_chunk.clone()])
    );
    assert!(activation.demand_roots[&a]
        .demand
        .binding_nodes
        .contains(&a_node));
    assert!(!activation.demand_roots[&a]
        .demand
        .binding_nodes
        .contains(&b_node));
    for projection in activation.demand_roots.values() {
        assert!(projection
            .demand
            .evaluation_modules
            .contains(Path::new("barrel.ts")));
    }
    assert_eq!(
        manifest.chunk_dependencies,
        BTreeSet::from([
            ChunkDependency {
                from: a_chunk,
                to: shared_chunk.clone()
            },
            ChunkDependency {
                from: b_chunk,
                to: shared_chunk
            },
        ])
    );
    assert_eq!(
        manifest
            .synthetic_nodes
            .iter()
            .filter(|n| matches!(n, SyntheticNode::AsyncFacade(_)))
            .count(),
        1
    );

    let reordered = SourceGraph::from_owned(sources.iter().rev().map(|(p, s)| input(p, s)));
    let other = crate::chunks::partition_precise(
        &reordered,
        &[entry(&reordered, "entry.ts", "main")],
        &policy,
    )
    .unwrap();
    assert_eq!(
        manifest,
        diagnostic_manifest(&reordered, &other, &policy).unwrap()
    );
}
