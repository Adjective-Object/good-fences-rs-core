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
