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
                    for d in &graph.file_facts(file as u32).unwrap().dependencies {
                        if d.kind == DependencyKind::DynamicImport {
                            expected.insert(
                                RootId::Async(AsyncSiteId {
                                    module: format!("{file}.ts").into(),
                                    offset: d.id.0,
                                    candidate: None,
                                }),
                                d.specifier.trim_end_matches(".ts").parse().unwrap(),
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
            plan.reachability,
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
