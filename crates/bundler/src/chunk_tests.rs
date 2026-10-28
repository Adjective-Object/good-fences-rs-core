use super::partition_conservative as partition;
use super::*;
use crate::{retain, tests::input};
use source_graph::SegmentKey;

fn entry(file: u32) -> RootSeed {
    RootSeed {
        id: RootId::Entry("main".into()),
        evaluation_roots: vec![file],
        binding_roots: vec![],
    }
}

fn module(path: &str) -> NodeId {
    NodeId::Module(path.into())
}

#[test]
fn oracle_handles_chain_diamond_scc_and_empty_identity() {
    let graph = SourceGraph::from_owned([
        input("a.ts", "import 'b.ts'; import 'c.ts'; run();"),
        input("b.ts", "import 'd.ts';"),
        input("c.ts", "import 'd.ts';"),
        input("d.ts", "import 'a.ts'; import 'empty.ts';"),
        input("empty.ts", ""),
        input("dead.ts", "hidden();"),
    ]);
    let retention = retain(&graph, &[0], &[]).unwrap();
    let reached =
        reference_reachability(&graph, &retention, &EffectPolicy::default(), &[entry(0)]).unwrap();
    let nodes = &reached[&entry(0).id];
    for path in ["a.ts", "b.ts", "c.ts", "d.ts", "empty.ts"] {
        assert!(nodes.contains(&module(path)));
    }
    assert!(!nodes.contains(&module("dead.ts")));
    insta::assert_debug_snapshot!(reached);
}

#[test]
fn overlapping_roots_share_canonical_path_storage() {
    let prefix = "long-application-package-component\\".repeat(7);
    let paths: Vec<_> = (0..160)
        .map(|index| format!("{prefix}{index:04}.ts"))
        .collect();
    let sources: Vec<_> = (0..160)
        .map(|index| {
            if index < 32 {
                format!("import '{}'; import '{}';", paths[32], paths[159])
            } else if index < 159 {
                format!("import '{}'; import '{}';", paths[index + 1], paths[159])
            } else {
                // A shared cycle must neither duplicate membership nor lose roots.
                format!("import '{}'; run();", paths[32])
            }
        })
        .collect();
    // JS specifiers use escaped native separators, while graph paths stay native.
    let graph = SourceGraph::from_owned(
        paths
            .iter()
            .zip(&sources)
            .map(|(path, source)| input(path, &source.replace('\\', "\\\\"))),
    );
    let roots: Vec<_> = (0..32)
        .map(|file| RootSeed {
            id: RootId::Entry(format!("entry-{file:02}")),
            ..entry(file)
        })
        .collect();
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    let mut identities = HashSet::new();
    let mut memberships = 0;
    for (root, nodes) in &plan.reachability {
        let start = plan.roots[root].evaluation_roots[0] as usize;
        let modules: Vec<_> = nodes
            .iter()
            .filter_map(|node| match node {
                NodeId::Module(path) => Some(path),
                _ => None,
            })
            .collect();
        let expected: Vec<_> = std::iter::once(&paths[start])
            .chain(&paths[32..])
            .map(std::path::PathBuf::from)
            .collect();
        assert_eq!(modules, expected.iter().collect::<Vec<_>>());
        assert!(nodes.iter().eq(nodes.into_iter()));
        assert!(nodes.iter().rev().eq(nodes.into_iter().rev()));
        assert_eq!(nodes.len(), nodes.iter().count());
        assert!(!nodes.is_empty());
        for node in nodes {
            identities.insert(node as *const NodeId);
            memberships += 1;
            assert!(nodes.contains(node));
        }
        assert!(!nodes.contains(&module("absent.ts")));
    }
    let unique = plan
        .node_roots
        .values()
        .filter(|mask| !mask.is_empty())
        .count();
    assert_eq!(
        identities.len(),
        unique,
        "root overlap must not allocate another path-bearing identity"
    );
    assert!(memberships > 24 * unique);
    let legacy = reference_reachability(&graph, &plan.retention, &policy, &roots).unwrap();
    let legacy_identities: HashSet<_> = legacy
        .values()
        .flatten()
        .map(|node| node as *const NodeId)
        .collect();
    assert_eq!(legacy_identities.len(), memberships);
    for (root, nodes) in &plan.reachability {
        assert!(nodes.iter().eq(legacy[root].iter()));
    }
    eprintln!(
        "overlapping roots: memberships={memberships}, legacy path identities={}, compact path identities={unique}",
        legacy_identities.len()
    );
    let mut reversed = roots.clone();
    reversed.reverse();
    let other = compact_reachability(&graph, &plan.retention, &policy, &reversed).unwrap();
    assert_eq!(plan.reachability, other);
}

#[test]
fn compact_membership_and_equality_do_not_depend_on_other_roots() {
    let graph = SourceGraph::from_owned([input("a.ts", ""), input("b.ts", "")]);
    let policy = EffectPolicy::default();
    let retention = retain(&graph, &[0, 1], &[]).unwrap();
    let empty = RootSeed {
        id: RootId::Entry("empty".into()),
        evaluation_roots: vec![],
        binding_roots: vec![],
    };
    let additional = RootSeed {
        id: RootId::Entry("additional".into()),
        ..entry(1)
    };
    let first =
        compact_reachability(&graph, &retention, &policy, &[entry(0), empty.clone()]).unwrap();
    let second =
        compact_reachability(&graph, &retention, &policy, &[entry(0), additional]).unwrap();
    assert_eq!(first[&entry(0).id], second[&entry(0).id]);
    assert!(first[&empty.id].is_empty());
    assert_eq!(first[&empty.id].iter().len(), 0);
    for path in ["a.ts", "b.ts", "absent.ts"] {
        assert!(!first[&empty.id].contains(&module(path)));
    }
    assert!(!second[&entry(0).id].contains(&module("b.ts")));
}

#[test]
fn individual_async_sites_do_not_carry_parent_roots_or_activate_dead_sites() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "import 'shared.ts'; import('shared.ts'); import('shared.ts'); function dead() { return import('dead.ts'); }"),
        input("shared.ts", "shared();"),
        input("dead.ts", "hidden();"),
    ]);
    let retention = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(retention.async_boundaries.len(), 2);
    let roots: Vec<_> = std::iter::once(entry(0))
        .chain(
            retention
                .async_boundaries
                .iter()
                .map(|&i| async_seed(&graph, i).unwrap()),
        )
        .collect();
    assert_ne!(roots[1].id, roots[2].id);
    let reached =
        reference_reachability(&graph, &retention, &EffectPolicy::default(), &roots).unwrap();
    for root in &roots {
        assert!(reached[&root.id].contains(&module("shared.ts")));
        assert!(!reached[&root.id].contains(&module("dead.ts")));
    }
    assert!(!reached[&roots[1].id].contains(&module("entry.ts")));
    insta::assert_debug_snapshot!(reached);
    let partitioned = partition(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    assert_eq!(
        partitioned.node_roots[&module("shared.ts")],
        roots.iter().map(|r| r.id.clone()).collect()
    );
    assert_exact_partition(&partitioned);

    let async_only = SourceGraph::from_owned([
        input("entry.ts", "import('target.ts');"),
        input("target.ts", "target();"),
    ]);
    let plan = retain(&async_only, &[0, 1], &[]).unwrap();
    let reached =
        reference_reachability(&async_only, &plan, &EffectPolicy::default(), &[entry(0)]).unwrap();
    assert!(!reached[&entry(0).id].contains(&module("target.ts")));
}

#[test]
fn oracle_rejects_stale_policy_revision_duplicate_and_unretained_roots() {
    let mut graph = SourceGraph::from_owned([input("entry.ts", "run();"), input("dead.ts", "")]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    let policy = EffectPolicy::default();
    assert!(reference_reachability(&graph, &plan, &policy, &[entry(1)]).is_err());
    assert!(reference_reachability(&graph, &plan, &policy, &[entry(0), entry(0)]).is_err());
    assert!(reference_reachability(
        &graph,
        &plan,
        &EffectPolicy::parse(["run(...)"]).unwrap(),
        &[entry(0)]
    )
    .is_err());
    assert!(async_seed(&graph, 99).is_err());
    assert!(node_id(
        &graph,
        GraphNode::Unlinked {
            file_id: 0,
            dependency: ast_segmenter::owned::DependencyId(999)
        }
    )
    .is_err());
    let mut invalid = entry(0);
    invalid
        .binding_roots
        .push(GraphNode::Segment(SegmentKey::new(0, 99)));
    assert!(reference_reachability(&graph, &plan, &policy, &[invalid]).is_err());
    graph
        .patch_owned(
            std::path::Path::new("entry.ts"),
            input("entry.ts", "changed();"),
        )
        .unwrap();
    assert!(reference_reachability(&graph, &plan, &policy, &[entry(0)]).is_err());
}

fn assert_exact_partition(plan: &ChunkPlan) {
    for (left, left_roots) in &plan.node_roots {
        for (right, right_roots) in &plan.node_roots {
            if !left_roots.is_empty() && !right_roots.is_empty() {
                assert_eq!(
                    left_roots == right_roots,
                    plan.chunk_for(left) == plan.chunk_for(right)
                );
            }
        }
        assert_eq!(plan.chunk_for(left).is_none(), left_roots.is_empty());
    }
    for (chunk, nodes) in &plan.chunks {
        assert!(!chunk.0.is_empty());
        assert!(nodes.iter().all(|node| plan.node_roots[node] == chunk.0));
    }
}

#[test]
fn nested_boundaries_reach_fixed_point_without_eager_or_dead_roots() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "import 'empty.ts'; import('nested.ts'); function dead() { return import('dead.ts'); }",
        ),
        input("nested.ts", "import('leaf.ts');"),
        input("leaf.ts", "import('nested.ts');"),
        input("empty.ts", ""),
        input("dead.ts", "hidden();"),
    ]);
    let plan = partition(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    assert_eq!(plan.roots.len(), 4);
    assert_eq!(
        plan.node_roots[&module("empty.ts")],
        BTreeSet::from([entry(0).id.clone()])
    );
    assert!(plan.node_roots[&module("dead.ts")].is_empty());
    assert!(!plan.node_roots[&module("leaf.ts")].contains(&entry(0).id));
    assert_exact_partition(&plan);
    insta::assert_debug_snapshot!((&plan.node_roots, &plan.chunks));
}

#[test]
fn namespace_binding_edges_and_entry_binding_roots_include_pure_declarations() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "import * as ns from 'lib.ts'; use(ns);"),
        input("lib.ts", "export const value = 1;"),
    ]);
    let mut root = entry(0);
    root.id = RootId::Entry("namespace".into());
    root.evaluation_roots.clear();
    root.binding_roots.push(GraphNode::Module(1));
    let plan = partition(&graph, &[entry(0), root.clone()], &EffectPolicy::default()).unwrap();
    assert_eq!(
        plan.node_roots[&NodeId::Segment {
            module: "lib.ts".into(),
            index: 0
        }],
        BTreeSet::from([entry(0).id, root.id])
    );
    assert_exact_partition(&plan);
}

#[test]
fn binding_queue_namespace_work_is_linear_in_edges_and_demands() {
    let graph = SourceGraph::from_owned([input("entry.ts", "")]);
    let mut measurements = Vec::new();
    for width in [64, 128] {
        let mut retention = retain(&graph, &[0], &[]).unwrap();
        retention.edges.clear();
        for file in 0..width {
            for kind in std::iter::once(RetentionEdgeKind::Namespace)
                .chain(std::iter::repeat_n(RetentionEdgeKind::Binding, 8))
            {
                retention.edges.push(crate::RetentionEdge {
                    from: GraphNode::Module(file),
                    to: GraphNode::Segment(SegmentKey::new(file, 0)),
                    kind,
                    source_edge: None,
                });
            }
        }
        let namespaces = NamespaceEdges::new(&retention);
        let mut queue = VecDeque::new();
        let demands = 16 * width as usize;
        for _root in 0..16 {
            for file in 0..width {
                enqueue_binding(&namespaces, GraphNode::Module(file), &mut queue);
                assert_eq!(
                    queue.drain(..).collect::<Vec<_>>(),
                    [
                        GraphNode::Module(file),
                        GraphNode::Segment(SegmentKey::new(file, 0))
                    ]
                );
            }
        }
        measurements.push((
            width,
            namespaces.edge_visits.get(),
            retention.edges.len() + demands,
        ));
    }
    eprintln!("binding namespace edge visits (width, actual, bound): {measurements:?}");
    for (width, actual, bound) in measurements {
        assert!(actual <= bound, "width {width}: {actual} > {bound}");
    }
}

#[test]
fn binding_queue_preserves_fifo_duplicates_cycles_and_repeated_demand() {
    let graph = SourceGraph::from_owned([input("entry.ts", "")]);
    let mut retention = retain(&graph, &[0], &[]).unwrap();
    let m = GraphNode::Module;
    let s = |file| GraphNode::Segment(SegmentKey::new(file, 0));
    retention.edges = [
        (m(0), m(1), RetentionEdgeKind::Namespace),
        (m(0), s(0), RetentionEdgeKind::Namespace),
        (m(0), s(99), RetentionEdgeKind::Binding),
        (m(0), m(2), RetentionEdgeKind::Namespace),
        (m(0), m(1), RetentionEdgeKind::Namespace),
        (m(1), m(2), RetentionEdgeKind::Namespace),
        (m(1), s(1), RetentionEdgeKind::Namespace),
        (s(0), s(99), RetentionEdgeKind::Namespace),
        (m(2), m(0), RetentionEdgeKind::Namespace),
        (m(2), s(2), RetentionEdgeKind::Namespace),
    ]
    .map(|(from, to, kind)| crate::RetentionEdge {
        from,
        to,
        kind,
        source_edge: None,
    })
    .into();
    let namespaces = NamespaceEdges::new(&retention);
    let expected = [m(0), m(1), s(0), m(2), m(1), m(2), s(1), m(0), s(2)];
    let mut queue = VecDeque::from([s(99)]);
    enqueue_binding(&namespaces, m(0), &mut queue);
    enqueue_binding(&namespaces, m(0), &mut queue);
    assert_eq!(
        queue.drain(..).collect::<Vec<_>>(),
        std::iter::once(s(99))
            .chain(expected)
            .chain(expected)
            .collect::<Vec<_>>()
    );
    enqueue_binding(&namespaces, m(0), &mut queue);
    assert_eq!(queue.drain(..).collect::<Vec<_>>(), expected);
    enqueue_binding(&namespaces, s(0), &mut queue);
    assert_eq!(queue.into_iter().collect::<Vec<_>>(), [s(0)]);
}

#[test]
fn binding_queue_keeps_evaluation_selected_and_namespace_roots_independent() {
    let graph = SourceGraph::from_owned([
        input(
            "barrel.ts",
            "export { first } from 'first.ts'; export { second } from 'second.ts';",
        ),
        input("first.ts", "export const first = 1;"),
        input("second.ts", "export const second = 2;"),
    ]);
    let namespace = RootSeed {
        id: RootId::Entry("namespace".into()),
        evaluation_roots: vec![],
        binding_roots: vec![GraphNode::Module(0)],
    };
    let selected = RootSeed {
        id: RootId::Entry("selected".into()),
        evaluation_roots: vec![0],
        binding_roots: vec![GraphNode::Segment(SegmentKey::new(1, 0))],
    };
    let retention = retain(&graph, &[0], &namespace.binding_roots).unwrap();
    let policy = EffectPolicy::default();
    let roots = [entry(0), selected.clone(), namespace.clone()];
    let reached = reference_reachability(&graph, &retention, &policy, &roots).unwrap();
    for (root, first, second) in [
        (entry(0).id, false, false),
        (selected.id, true, false),
        (namespace.id, true, true),
    ] {
        assert_eq!(reached[&root].contains(&segment("first.ts", 0)), first);
        assert_eq!(reached[&root].contains(&segment("second.ts", 0)), second);
    }
    let reversed = roots.into_iter().rev().collect::<Vec<_>>();
    assert_eq!(
        reached,
        reference_reachability(&graph, &retention, &policy, &reversed).unwrap()
    );
}

#[test]
fn tombstones_empty_modules_and_policy_changes_do_not_reuse_plans() {
    let mut graph = SourceGraph::from_owned([
        input("removed.ts", ""),
        input("entry.ts", "import('empty.ts');"),
        input("empty.ts", ""),
    ]);
    assert!(graph.remove_file(std::path::Path::new("removed.ts")));
    assert_eq!(graph.module_ids().collect::<Vec<_>>(), [1, 2]);
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &[entry(1)], &policy).unwrap();
    assert!(plan.node_roots.contains_key(&module("empty.ts")));
    assert!(!plan.node_roots.contains_key(&module("removed.ts")));
    assert!(plan.is_current(&graph, &policy));
    assert!(!plan.is_current(&graph, &policy.clone().with_pure_annotations(false)));
    graph
        .patch_owned(
            std::path::Path::new("empty.ts"),
            input("empty.ts", "changed();"),
        )
        .unwrap();
    assert!(!plan.is_current(&graph, &policy));
    assert!(partition(&graph, &[entry(1), entry(1)], &policy).is_err());
    assert!(partition(
        &graph,
        &[RootSeed {
            id: RootId::Async(AsyncSiteId {
                module: "x".into(),
                offset: 0,
                candidate: None,
            }),
            ..entry(1)
        }],
        &policy
    )
    .is_err());
    assert!(partition(&graph, &[], &policy).unwrap().chunks.is_empty());
}

fn random(state: &mut u64) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state as usize
}

fn selected_root(graph: &SourceGraph, consumer: &str, name: &str) -> RootId {
    let file = graph.file_id(std::path::Path::new(consumer)).unwrap();
    let dependency = graph
        .file_facts(file)
        .unwrap()
        .dependencies()
        .iter()
        .find(|d| d.kind() == ast_segmenter::owned::DependencyKind::DynamicImport)
        .unwrap()
        .id();
    RootId::AsyncExport {
        site: AsyncSiteId {
            module: consumer.into(),
            offset: dependency.0,
            candidate: None,
        },
        name: name.into(),
    }
}

fn segment(path: &str, index: u32) -> NodeId {
    NodeId::Segment {
        module: path.into(),
        index,
    }
}

#[test]
fn precise_same_occurrence_has_independent_export_masks_and_shared_original_effects() {
    let sources = [
                        ("entry.ts", "const {a, b} = await import('barrel.ts'); use(a, b);"),
                        ("barrel.ts", "import 'effects.ts'; markBarrel(); export {a, b, unused} from 'leaf.ts';"),
                        ("effects.ts", "function effectHelper() { markEffect(); } effectHelper();"),
                        ("leaf.ts", "function shared() { return 1; } function onlyA() { return 2; } function onlyB() { return 3; } export function a() { return shared() + onlyA(); } export function b() { return shared() + onlyB(); } export function unused() { return 4; }"),
                    ];
    let graph = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
    let policy = EffectPolicy::default();
    let plan = partition_precise(&graph, &[entry(0)], &policy).unwrap();
    let a = selected_root(&graph, "entry.ts", "a");
    let b = selected_root(&graph, "entry.ts", "b");
    let both = BTreeSet::from([a.clone(), b.clone()]);
    assert_eq!(plan.roots.len(), 3);
    assert_eq!(a.async_site(), b.async_site());
    assert_eq!(plan.async_activations.len(), 1);
    assert_eq!(plan.async_activations[a.async_site().unwrap()].roots, both);
    for root in [&a, &b] {
        assert_eq!(plan.roots[root].evaluation_roots, [1]);
        assert!(!plan.roots[root]
            .binding_roots
            .contains(&GraphNode::Module(1)));
        assert!(plan.reachability[root].contains(&module("barrel.ts")));
        assert!(!plan.reachability[root].contains(&module("entry.ts")));
    }
    for node in [segment("leaf.ts", 1), segment("leaf.ts", 3)] {
        assert_eq!(plan.node_roots[&node], BTreeSet::from([a.clone()]));
    }
    for node in [segment("leaf.ts", 2), segment("leaf.ts", 4)] {
        assert_eq!(plan.node_roots[&node], BTreeSet::from([b.clone()]));
    }
    for node in [
        segment("leaf.ts", 0),
        segment("barrel.ts", 1),
        segment("barrel.ts", 2),
        segment("effects.ts", 0),
        segment("effects.ts", 1),
        module("barrel.ts"),
        module("effects.ts"),
    ] {
        assert_eq!(plan.node_roots[&node], both, "{node:?}");
    }
    assert!(plan.node_roots[&segment("leaf.ts", 5)].is_empty());
    assert_exact_partition(&plan);
    let reordered = SourceGraph::from_owned(sources.iter().rev().map(|(p, s)| input(p, s)));
    let other = partition_precise(
        &reordered,
        &[entry(
            reordered.file_id(std::path::Path::new("entry.ts")).unwrap(),
        )],
        &policy,
    )
    .unwrap();
    assert_eq!(plan.node_roots, other.node_roots);
    assert_eq!(plan.chunks, other.chunks);
}

#[test]
fn precise_requested_aliases_are_distinct_roots_even_for_the_same_canonical_binding() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const {chosen, other} = await import('barrel.ts'); use(chosen, other);",
        ),
        input(
            "barrel.ts",
            "export {value as chosen, value as other} from 'leaf.ts';",
        ),
        input("leaf.ts", "export const value = 1; export const spare = 2;"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let chosen = selected_root(&graph, "entry.ts", "chosen");
    let other = selected_root(&graph, "entry.ts", "other");
    assert_eq!(plan.roots.len(), 3);
    assert_eq!(
        plan.roots[&chosen].binding_roots,
        plan.roots[&other].binding_roots
    );
    assert_eq!(
        plan.node_roots[&segment("leaf.ts", 0)],
        BTreeSet::from([chosen, other])
    );
    assert!(plan.node_roots[&segment("leaf.ts", 1)].is_empty());
}

#[test]
fn precise_public_and_namespace_roots_still_preserve_the_complete_surface() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const a = (await import('barrel.ts')).a; use(a);",
        ),
        input("barrel.ts", "export {a, b} from 'leaf.ts';"),
        input("leaf.ts", "export const a = 1; export const b = 2;"),
        input("full.ts", "import('barrel.ts').then(consume);"),
        input("static.ts", "import {a} from 'barrel.ts'; use(a);"),
    ]);
    let public = RootSeed {
        id: RootId::Entry("public".into()),
        evaluation_roots: vec![],
        binding_roots: vec![GraphNode::Module(1)],
    };
    let full = RootSeed {
        id: RootId::Entry("full".into()),
        ..entry(3)
    };
    let static_root = RootSeed {
        id: RootId::Entry("static".into()),
        ..entry(4)
    };
    let plan = partition_precise(
        &graph,
        &[entry(0), public.clone(), full, static_root.clone()],
        &EffectPolicy::default(),
    )
    .unwrap();
    let full_root = plan
        .roots
        .keys()
        .find(
            |r| matches!(r, RootId::Async(site) if site.module == std::path::Path::new("full.ts")),
        )
        .unwrap()
        .clone();
    assert_eq!(
        plan.node_roots[&segment("leaf.ts", 0)],
        BTreeSet::from([
            public.id.clone(),
            full_root.clone(),
            static_root.id,
            selected_root(&graph, "entry.ts", "a")
        ])
    );
    assert_eq!(
        plan.node_roots[&segment("leaf.ts", 1)],
        BTreeSet::from([public.id, full_root])
    );
}

#[test]
fn precise_nested_discovery_does_not_activate_dead_exports_or_cross_async_edges() {
    let graph = SourceGraph::from_owned([
                        input("entry.ts", "const go = (await import('barrel.ts')).go; use(go);"),
                        input("barrel.ts", "export {go, dead} from 'outer.ts';"),
                        input("outer.ts", "export function go() { return import('nested.ts').then(ns => ns.chosen); } export function dead() { return import('dead.ts'); }"),
                        input("nested.ts", "export const chosen = 1; export const spare = 2;"),
                        input("dead.ts", "deadEffect();"),
                    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let go = selected_root(&graph, "entry.ts", "go");
    let nested = selected_root(&graph, "outer.ts", "chosen");
    assert_eq!(
        plan.roots.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([entry(0).id, go.clone(), nested.clone()])
    );
    assert_eq!(
        plan.node_roots[&segment("outer.ts", 0)],
        BTreeSet::from([go])
    );
    assert_eq!(
        plan.node_roots[&segment("nested.ts", 0)],
        BTreeSet::from([nested])
    );
    for node in [
        segment("outer.ts", 1),
        segment("nested.ts", 1),
        module("dead.ts"),
        segment("dead.ts", 0),
    ] {
        assert!(plan.node_roots[&node].is_empty(), "{node:?}");
    }
    assert_exact_partition(&plan);
}

#[test]
fn precise_missing_properties_and_unused_results_are_evaluation_only() {
    for source in [
        "const absent = (await import('target.ts')).absent; use(absent);",
        "import('target.ts').then(ns => ns.absent);",
        "import('target.ts');",
    ] {
        let graph = SourceGraph::from_owned([
            input("entry.ts", source),
            input("target.ts", "effect(); export const spare = 1;"),
        ]);
        let policy = EffectPolicy::default();
        let plan = partition_precise(&graph, &[entry(0)], &policy).unwrap();
        let (root, seed) = plan
            .roots
            .iter()
            .find(|(r, _)| matches!(r, RootId::Async(_)))
            .unwrap();
        assert_eq!(seed.evaluation_roots, [1]);
        assert!(seed.binding_roots.is_empty());
        assert_eq!(
            plan.node_roots[&segment("target.ts", 0)],
            BTreeSet::from([root.clone()])
        );
        assert!(plan.node_roots[&segment("target.ts", 1)].is_empty());
        let compatible = partition(&graph, &[entry(0)], &policy).unwrap();
        assert!(!compatible.node_roots[&segment("target.ts", 1)].is_empty());
    }
}

#[test]
fn precise_unsafe_observations_and_then_exports_keep_full_namespace_masks() {
    for (source, target) in [
                        ("import('target.ts').then(consume);", "export const a = 1; export const spare = 2;"),
                        ("const ns = await import('target.ts'); use(ns[key]);", "export const a = 1; export const spare = 2;"),
                        ("(await import('target.ts')).a();", "export function a() { return this; } export const spare = 2;"),
                        ("const a = (await import('target.ts')).a; use(a);", "export const a = 1; export const spare = 2; export function then(resolve) { resolve(1); }"),
                    ] {
                        let graph = SourceGraph::from_owned([input("entry.ts", source), input("target.ts", target)]);
                        let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
                        let (root, seed) = plan.roots.iter().find(|(r, _)| matches!(r, RootId::Async(_))).unwrap();
                        assert_eq!(plan.roots.len(), 2, "{source}");
                        assert_eq!(seed.binding_roots, [GraphNode::Module(1)]);
                        for index in 0..graph.file_segments(1).unwrap().len() {
                            assert_eq!(plan.node_roots[&segment("target.ts", index as u32)], BTreeSet::from([root.clone()]), "{source}");
                        }
                    }
}

#[test]
fn precise_routes_through_star_and_local_export_connectors_do_not_widen_bindings() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const {a, b} = await import('star.ts'); use(a, b);",
        ),
        input("star.ts", "export * from 'local.ts';"),
        input(
            "local.ts",
            "import {a, b, unused} from 'leaf.ts'; export {a, b, unused};",
        ),
        input(
            "leaf.ts",
            "export const a = 1; export const b = 2; export const unused = 3;",
        ),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let a = selected_root(&graph, "entry.ts", "a");
    let b = selected_root(&graph, "entry.ts", "b");
    assert_eq!(
        plan.node_roots[&segment("leaf.ts", 0)],
        BTreeSet::from([a.clone()])
    );
    assert_eq!(
        plan.node_roots[&segment("leaf.ts", 1)],
        BTreeSet::from([b.clone()])
    );
    assert!(plan.node_roots[&segment("leaf.ts", 2)].is_empty());
    assert_eq!(
        plan.node_roots[&segment("star.ts", 0)],
        BTreeSet::from([a.clone(), b.clone()])
    );
    assert_eq!(
        plan.node_roots[&segment("local.ts", 1)],
        BTreeSet::from([a, b])
    );
}

#[test]
fn precise_distinct_occurrences_of_the_same_export_keep_distinct_identities() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "const a = (await import('target.ts')).value; const b = (await import('target.ts')).value; use(a, b);"),
        input("target.ts", "export const value = 1; export const spare = 2;"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    assert_eq!(plan.async_activations.len(), 2);
    let roots: BTreeSet<_> = plan
        .roots
        .keys()
        .filter(|root| matches!(root, RootId::AsyncExport {name, ..} if name == "value"))
        .cloned()
        .collect();
    assert_eq!(roots.len(), 2);
    assert_eq!(plan.node_roots[&segment("target.ts", 0)], roots);
    assert!(plan.node_roots[&segment("target.ts", 1)].is_empty());
}

#[test]
fn precise_imported_helpers_follow_each_use_not_all_specifiers_in_the_declaration() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "const {a, b} = await import('target.ts'); use(a, b);"),
        input("target.ts", "import {shared, onlyA, onlyB, unused} from 'helpers.ts'; export function a() { return shared() + onlyA(); } export function b() { return shared() + onlyB(); }"),
        input("helpers.ts", "export function shared() { return 1; } export function onlyA() { return 2; } export function onlyB() { return 3; } export function unused() { return 4; }"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let a = selected_root(&graph, "entry.ts", "a");
    let b = selected_root(&graph, "entry.ts", "b");
    assert_eq!(
        plan.node_roots[&segment("helpers.ts", 0)],
        BTreeSet::from([a.clone(), b.clone()])
    );
    assert_eq!(
        plan.node_roots[&segment("helpers.ts", 1)],
        BTreeSet::from([a])
    );
    assert_eq!(
        plan.node_roots[&segment("helpers.ts", 2)],
        BTreeSet::from([b])
    );
    assert!(plan.node_roots[&segment("helpers.ts", 3)].is_empty());
}

#[test]
fn precise_local_commonjs_exports_preserve_full_namespace_and_public_snapshots() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "import('barrel.ts').then(consume);"),
        input(
            "barrel.ts",
            "import {a, b, unused} from 'dep.cjs'; export {a, b};",
        ),
        input(
            "dep.cjs",
            "exports.a = 1; exports.b = 2; exports.unused = 3;",
        ),
    ]);
    let policy = EffectPolicy::default();
    let mut public = entry(1);
    public.binding_roots.push(GraphNode::Module(1));
    for roots in [vec![entry(0)], vec![public]] {
        let conservative = partition(&graph, &roots, &policy).unwrap();
        let precise = partition_precise(&graph, &roots, &policy).unwrap();
        for name in ["a", "b"] {
            let snapshot = NodeId::CommonJsProxy {
                module: "dep.cjs".into(),
                name: Some(name.into()),
            };
            assert!(!conservative.node_roots[&snapshot].is_empty());
            assert_eq!(
                precise.node_roots.get(&snapshot),
                conservative.node_roots.get(&snapshot)
            );
        }
        assert!(!precise.node_roots.contains_key(&NodeId::CommonJsProxy {
            module: "dep.cjs".into(),
            name: Some("unused".into()),
        }));
        assert_exact_partition(&precise);
    }
}

#[test]
fn precise_selected_local_commonjs_exports_keep_the_conservative_surface() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const {a, b} = await import('barrel.ts'); use(a, b);",
        ),
        input(
            "barrel.ts",
            "import {a as first, b as second, unused} from 'dep.cjs'; export {first as a, second as b, unused};",
        ),
        input("dep.cjs", "exports.a = 1; exports.b = 2; exports.unused = 3;"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let root = plan
        .roots
        .keys()
        .find(|root| matches!(root, RootId::Async(_)))
        .unwrap();
    assert!(!plan
        .roots
        .keys()
        .any(|root| matches!(root, RootId::AsyncExport { .. })));
    for name in ["a", "b", "unused"] {
        assert_eq!(
            plan.node_roots.get(&NodeId::CommonJsProxy {
                module: "dep.cjs".into(),
                name: Some(name.into()),
            }),
            Some(&BTreeSet::from([root.clone()]))
        );
    }
    assert_eq!(
        plan.node_roots[&module("barrel.ts")],
        BTreeSet::from([root.clone()])
    );
    assert_exact_partition(&plan);
}

#[test]
fn precise_selected_esm_export_does_not_demand_a_sibling_commonjs_snapshot() {
    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const b = (await import('barrel.ts')).b; use(b);",
        ),
        input(
            "barrel.ts",
            "import {a} from 'dep.cjs'; const b = 2; export {a, b};",
        ),
        input("dep.cjs", "exports.a = 1;"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &EffectPolicy::default()).unwrap();
    let b = selected_root(&graph, "entry.ts", "b");
    assert_eq!(
        plan.node_roots[&segment("barrel.ts", 1)],
        BTreeSet::from([b.clone()])
    );
    assert_eq!(plan.node_roots[&module("barrel.ts")], BTreeSet::from([b]));
    assert!(!plan.node_roots.contains_key(&NodeId::CommonJsProxy {
        module: "dep.cjs".into(),
        name: Some("a".into()),
    }));
    assert_exact_partition(&plan);
}

#[test]
fn precise_context_commonjs_and_external_targets_keep_conservative_roots() {
    let mut source = input(
        "entry.ts",
        "const a = (await import(`./${name}.ts`)).a; use(a);",
    );
    let dependency = source.facts.dependencies().get(0).unwrap().id();
    source.resolutions.insert(
        dependency,
        source_graph::DependencyTarget::Context(BTreeMap::from([
            ("./one.ts".into(), PathBuf::from("one.ts")),
            ("./two.ts".into(), PathBuf::from("two.ts")),
        ])),
    );
    let graph = SourceGraph::from_owned([
        source,
        input("one.ts", "export const a = 1; export const spare = 2;"),
        input("two.ts", "export const a = 3; export const spare = 4;"),
    ]);
    let policy = EffectPolicy::default();
    let plan = partition_precise(&graph, &[entry(0)], &policy).unwrap();
    assert_eq!(plan.async_activations.len(), 2);
    for target in ["one.ts", "two.ts"] {
        let root = RootId::Async(AsyncSiteId {
            module: "entry.ts".into(),
            offset: dependency.0,
            candidate: Some(target.into()),
        });
        assert!(plan.roots.contains_key(&root));
        for index in 0..2 {
            assert_eq!(
                plan.node_roots[&segment(target, index)],
                BTreeSet::from([root.clone()])
            );
        }
    }
    let manifest = crate::manifest::diagnostic_manifest(&graph, &plan, &policy).unwrap();
    assert_eq!(manifest.async_facades.len(), 2);
    for (site, activation) in &manifest.async_facades {
        assert_eq!(
            activation.target,
            NodeId::Module(site.candidate.clone().unwrap())
        );
        assert_eq!(activation.demand_roots.len(), 1);
    }

    let graph = SourceGraph::from_owned([
        input(
            "entry.ts",
            "const a = (await import('opaque.cjs')).a; use(a);",
        ),
        input("opaque.cjs", "exports.a = 1; exports.spare = 2;"),
    ]);
    let plan = partition_precise(&graph, &[entry(0)], &policy).unwrap();
    let root = plan
        .roots
        .keys()
        .find(|r| matches!(r, RootId::Async(_)))
        .unwrap();
    assert_eq!(plan.roots[root].binding_roots, [GraphNode::Module(1)]);
    for index in 0..graph.file_segments(1).unwrap().len() {
        assert_eq!(
            plan.node_roots[&segment("opaque.cjs", index as u32)],
            BTreeSet::from([root.clone()])
        );
    }
    assert_eq!(
        plan.node_roots[&NodeId::CommonJsNamespace("opaque.cjs".into())],
        BTreeSet::from([root.clone()])
    );

    let mut source = input(
        "entry.ts",
        "const a = (await import('external')).a; use(a);",
    );
    let dependency = source.facts.dependencies().get(0).unwrap().id();
    source.resolutions.insert(
        dependency,
        source_graph::DependencyTarget::External("external".into()),
    );
    let graph = SourceGraph::from_owned([source]);
    let plan = partition_precise(&graph, &[entry(0)], &policy).unwrap();
    let root = plan
        .roots
        .keys()
        .find(|r| matches!(r, RootId::Async(_)))
        .unwrap();
    assert_eq!(
        plan.node_roots[&NodeId::External {
            module: "entry.ts".into(),
            offset: dependency.0
        }],
        BTreeSet::from([root.clone()])
    );
    let manifest = crate::manifest::diagnostic_manifest(&graph, &plan, &policy).unwrap();
    let activation = manifest.async_facades.values().next().unwrap();
    assert!(matches!(activation.target, NodeId::External { .. }));
    assert!(activation.facade.demand.evaluation_modules.is_empty());
}

#[test]
fn randomized_small_graphs_match_module_oracle_and_insertion_order() {
    use ast_segmenter::owned::DependencyKind;
    let policy = EffectPolicy::default();
    let mut state = 0xcafe_1234_5678;
    for case in 0..48 {
        let count = 2 + random(&mut state) % 5;
        let mut sources = Vec::new();
        let mut synchronous = vec![Vec::new(); count];
        for (from, dependencies) in synchronous.iter_mut().enumerate() {
            let mut source = String::from("run();");
            for to in 0..count {
                if random(&mut state).is_multiple_of(5) {
                    source.push_str(&format!("import '{to}.ts';"));
                    dependencies.push(to);
                }
                if random(&mut state).is_multiple_of(6) {
                    source.push_str(&format!("import('{to}.ts');"));
                }
            }
            sources.push((format!("{from}.ts"), source));
        }
        let graph = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
        let mut roots = vec![
            entry(0),
            RootSeed {
                id: RootId::Entry("second".into()),
                ..entry(1)
            },
        ];
        let plan = partition(&graph, &roots, &policy).unwrap();
        let mut expected =
            BTreeMap::from([(roots[0].id.clone(), 0usize), (roots[1].id.clone(), 1usize)]);
        let mut visited = BTreeMap::new();
        loop {
            let old = expected.len();
            for (root, &start) in &expected.clone() {
                let mut queue = vec![start];
                let mut files = BTreeSet::new();
                while let Some(file) = queue.pop() {
                    if files.insert(file) {
                        queue.extend(&synchronous[file]);
                    }
                }
                for &file in &files {
                    for d in graph.file_facts(file as u32).unwrap().dependencies() {
                        if d.kind() == DependencyKind::DynamicImport {
                            expected.insert(
                                RootId::Async(AsyncSiteId {
                                    module: format!("{file}.ts").into(),
                                    offset: d.id().0,
                                    candidate: None,
                                }),
                                d.specifier().trim_end_matches(".ts").parse().unwrap(),
                            );
                        }
                    }
                }
                visited.insert(
                    root.clone(),
                    files
                        .into_iter()
                        .map(|f| module(&format!("{f}.ts")))
                        .collect::<BTreeSet<_>>(),
                );
            }
            if old == expected.len() {
                break;
            }
        }
        for (root, nodes) in &plan.reachability {
            let modules: BTreeSet<_> = nodes
                .iter()
                .filter(|n| matches!(n, NodeId::Module(_)))
                .cloned()
                .collect();
            assert_eq!(modules, visited[root], "case {case}: {root:?}");
        }
        assert_eq!(plan.roots.len(), expected.len());
        assert_exact_partition(&plan);
        let mut order: Vec<_> = (0..count).collect();
        for i in (1..count).rev() {
            let other = random(&mut state) % (i + 1);
            order.swap(i, other);
        }
        let reordered = SourceGraph::from_owned(
            order
                .into_iter()
                .map(|i| input(&sources[i].0, &sources[i].1)),
        );
        roots.reverse();
        for root in &mut roots {
            let path = match &root.id {
                RootId::Entry(id) if id == "main" => "0.ts",
                _ => "1.ts",
            };
            root.evaluation_roots = vec![reordered.file_id(std::path::Path::new(path)).unwrap()];
        }
        let other = partition(&reordered, &roots, &policy).unwrap();
        assert_eq!(plan.node_roots, other.node_roots, "case {case}");
        assert_eq!(plan.chunks, other.chunks, "case {case}");
        let mut shuffled = plan.retention.clone();
        for i in (1..shuffled.edges.len()).rev() {
            let other = random(&mut state) % (i + 1);
            shuffled.edges.swap(i, other);
        }
        assert_eq!(
            plan.reachability
                .iter()
                .map(|(root, nodes)| (root.clone(), nodes.iter().cloned().collect::<BTreeSet<_>>()))
                .collect::<BTreeMap<_, _>>(),
            reference_reachability(
                &graph,
                &shuffled,
                &policy,
                &plan.roots.values().cloned().collect::<Vec<_>>()
            )
            .unwrap()
        );
    }
}
