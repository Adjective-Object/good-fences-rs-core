//! Derived retention and order constraints. Pure statements remain connectors
//! until projection, but never become scheduling nodes just to preserve a seam.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use source_graph::{Activation, EdgeKind, EdgePurpose, GraphNode, SegmentKey, SourceGraph};

use crate::{effects::StatementEffects, RetentionEdge, RetentionEdgeKind};

type Dependencies = HashMap<GraphNode, Vec<GraphNode>>;

pub(crate) fn plan(
    graph: &SourceGraph,
    modules: &BTreeSet<u32>,
    roots: &[u32],
    effects: &BTreeMap<SegmentKey, StatementEffects>,
    edges: &mut HashSet<RetentionEdge>,
) -> (Vec<SegmentKey>, Vec<String>) {
    let effectful: HashSet<_> = effects
        .iter()
        .filter(|(_, summary)| summary.is_effectful())
        .map(|(&key, _)| GraphNode::Segment(key))
        .collect();
    let ordering = module_dependencies(graph, modules, roots, &effectful, edges);

    // Canonical path/span rank makes both SCC breaking and diagnostics independent
    // of graph insertion IDs. Never mix binding edges into this order graph.
    let mut nodes: Vec<_> = effectful.into_iter().collect();
    nodes.sort_by_key(|node| {
        let GraphNode::Segment(key) = node else {
            unreachable!()
        };
        (
            graph.file_path(key.file_id).expect("effect module"),
            effects[key].span.start,
            effects[key].span.end,
        )
    });
    let ranks: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, &node)| (node, i))
        .collect();
    let effectful = nodes.iter().copied().collect();
    let (projected, _) = project_all(&ordering, &nodes, &effectful);
    let mut dependencies: Vec<BTreeSet<usize>> = projected
        .into_iter()
        .map(|targets| targets.iter().map(|node| ranks[node]).collect())
        .collect();
    let components = components(&dependencies);
    let mut warnings = Vec::new();
    let mut order = Vec::new();
    for component in components {
        if component.len() > 1 || dependencies[component[0]].contains(&component[0]) {
            let chosen: Vec<_> = component
                .iter()
                .map(|&rank| {
                    let GraphNode::Segment(key) = nodes[rank] else {
                        unreachable!()
                    };
                    let span = effects[&key].span;
                    format!(
                        "{}:{}..{}",
                        graph.file_path(key.file_id).unwrap().display(),
                        span.start,
                        span.end,
                    )
                })
                .collect();
            warnings.push(format!(
                "effect-order cycle; source members in chosen order: {}",
                chosen.join(" -> ")
            ));
            let members: BTreeSet<_> = component.iter().copied().collect();
            for &rank in &component {
                dependencies[rank].retain(|target| !members.contains(target) || *target < rank);
            }
            for pair in component.windows(2) {
                dependencies[pair[1]].insert(pair[0]);
            }
        }
        order.extend(component.into_iter().map(|rank| {
            let GraphNode::Segment(key) = nodes[rank] else {
                unreachable!()
            };
            key
        }));
    }
    for (from, targets) in dependencies.iter().enumerate() {
        for &to in targets {
            edges.insert(RetentionEdge {
                from: nodes[from],
                to: nodes[to],
                kind: RetentionEdgeKind::EffectOrder,
                source_edge: None,
            });
        }
    }
    (order, warnings)
}

fn module_dependencies(
    graph: &SourceGraph,
    modules: &BTreeSet<u32>,
    roots: &[u32],
    effectful: &HashSet<GraphNode>,
    edges: &mut HashSet<RetentionEdge>,
) -> Dependencies {
    let mut ordering = Dependencies::new();
    let mut imports_by_module = HashMap::new();
    let mut effects_by_module = HashMap::new();
    for &file in modules {
        let module = GraphNode::Module(file);
        let segments = graph.file_segments(file).expect("evaluated module");
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut local_effects = Vec::new();
        for index in 0..segments.len() {
            let node = GraphNode::Segment(SegmentKey::new(file, index as u32));
            let mut is_import = false;
            for edge in graph.outgoing(node).filter(|edge| {
                matches!(edge.kind, EdgeKind::Import | EdgeKind::ReExport)
                    && edge.purpose != EdgePurpose::Binding
                    && edge.activation == Activation::Synchronous
                    && edge.enclosing_function.is_none()
                    && !edge.type_only
            }) {
                is_import = true;
                imports.push((node, edge.to));
                ordering.entry(node).or_default().push(edge.to);
            }
            if effectful.contains(&node) {
                local_effects.push(node);
                // BindingEvaluation already routes every live segment through this hub;
                // StaticEvaluation supplies the imported modules without a closure copy.
                edges.insert(RetentionEdge {
                    from: module,
                    to: node,
                    kind: RetentionEdgeKind::ModuleEffect,
                    source_edge: None,
                });
            }
            if !is_import {
                body.push(node);
            }
        }
        // Hoisted declarations form their own chain, never following executable code.
        let mut previous = None;
        for node in imports.iter().map(|&(node, _)| node).chain(body) {
            if let Some(previous) = previous.filter(|previous| *previous != node) {
                ordering.entry(node).or_default().push(previous);
            }
            previous = Some(node);
        }
        ordering.entry(module).or_default().extend(previous);
        imports_by_module.insert(file, imports);
        effects_by_module.insert(file, local_effects);
    }

    chain_static_evaluation(
        graph,
        modules,
        roots,
        &imports_by_module,
        &effects_by_module,
        &mut ordering,
    );

    ordering
}

/// Chain synchronous completions only. Async subtrees may suspend while later
/// siblings run; a scalar effect DAG is not a native async module scheduler.
fn chain_static_evaluation(
    graph: &SourceGraph,
    modules: &BTreeSet<u32>,
    roots: &[u32],
    imports: &HashMap<u32, Vec<(GraphNode, GraphNode)>>,
    effects: &HashMap<u32, Vec<GraphNode>>,
    ordering: &mut Dependencies,
) {
    let mut users: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&file, dependencies) in imports {
        for &(_, target) in dependencies {
            if let GraphNode::Module(target) = target {
                users.entry(target).or_default().push(file);
            }
        }
    }
    let mut asynchronous = HashSet::new();
    let mut pending: Vec<_> = modules
        .iter()
        .copied()
        .filter(|&file| {
            graph
                .file_facts(file)
                .is_some_and(|facts| !facts.top_level_await().is_empty())
        })
        .collect();
    while let Some(file) = pending.pop() {
        if asynchronous.insert(file) {
            pending.extend(users.get(&file).into_iter().flatten().copied());
        }
    }
    let mut roots = roots.to_vec();
    roots.sort_by_key(|&file| graph.file_path(file));
    roots.dedup();
    let mut remaining: Vec<_> = modules.iter().copied().collect();
    remaining.sort_by_key(|&file| graph.file_path(file));
    let mut seen = HashSet::new();
    for root in roots.into_iter().chain(remaining) {
        let mut previous = None;
        let mut stack = vec![(root, false)];
        while let Some((file, exiting)) = stack.pop() {
            if exiting {
                if asynchronous.contains(&file) {
                    continue;
                }
                for &effect in &effects[&file] {
                    if let Some(previous) = previous {
                        ordering.entry(effect).or_default().push(previous);
                    }
                    previous = Some(effect);
                }
            } else if seen.insert(file) {
                stack.push((file, true));
                stack.extend(imports[&file].iter().rev().filter_map(|&(_, target)| {
                    if let GraphNode::Module(target) = target {
                        Some((target, false))
                    } else {
                        None
                    }
                }));
            }
        }
    }
}

/// Cache first-effect projections, including empty results, per connector SCC.
/// Single-successor chains share sets rather than copying a closure per segment.
/// Work counts indexing/edge visits and projected targets merged/copied;
/// the two iterative SCC passes additionally visit each connector/edge once.
fn project_all(
    graph: &Dependencies,
    nodes: &[GraphNode],
    effectful: &HashSet<GraphNode>,
) -> (Vec<Arc<HashSet<GraphNode>>>, usize) {
    let mut work = 0;
    let mut connectors = HashMap::new();
    for node in graph.keys().chain(graph.values().flatten()) {
        work += 1;
        if !effectful.contains(node) {
            let next = connectors.len();
            connectors.entry(*node).or_insert(next);
        }
    }
    let mut dependencies = vec![BTreeSet::new(); connectors.len()];
    let mut terminals = vec![HashSet::new(); connectors.len()];
    for (&node, targets) in graph {
        let Some(&rank) = connectors.get(&node) else {
            continue;
        };
        for target in targets {
            work += 1;
            if effectful.contains(target) {
                terminals[rank].insert(*target);
            } else {
                dependencies[rank].insert(connectors[target]);
            }
        }
    }
    let components = components(&dependencies);
    let mut owners = vec![0; connectors.len()];
    for (owner, component) in components.iter().enumerate() {
        for &rank in component {
            owners[rank] = owner;
        }
    }
    let empty = Arc::new(HashSet::new());
    let mut cache = vec![Arc::clone(&empty); components.len()];
    for (owner, component) in components.iter().enumerate() {
        let mut projected = Arc::clone(&empty);
        for &rank in component {
            work += terminals[rank].len();
            if !terminals[rank].is_empty() {
                if Arc::strong_count(&projected) > 1 {
                    work += projected.len();
                }
                Arc::make_mut(&mut projected).extend(terminals[rank].iter().copied());
            }
            for &target in &dependencies[rank] {
                work += 1;
                let dependency = owners[target];
                if dependency != owner {
                    debug_assert!(dependency < owner);
                    merge_projection(&mut projected, &cache[dependency], &mut work);
                }
            }
        }
        cache[owner] = projected;
    }
    let projected = nodes
        .iter()
        .map(|node| {
            let mut projected = Arc::clone(&empty);
            for target in graph.get(node).into_iter().flatten() {
                work += 1;
                if effectful.contains(target) {
                    if Arc::strong_count(&projected) > 1 {
                        work += projected.len();
                    }
                    Arc::make_mut(&mut projected).insert(*target);
                } else {
                    merge_projection(
                        &mut projected,
                        &cache[owners[connectors[target]]],
                        &mut work,
                    );
                }
            }
            projected
        })
        .collect();
    (projected, work)
}

fn merge_projection(
    targets: &mut Arc<HashSet<GraphNode>>,
    next: &Arc<HashSet<GraphNode>>,
    work: &mut usize,
) {
    if targets.is_empty() {
        *targets = Arc::clone(next);
    } else if !next.is_empty() && !Arc::ptr_eq(targets, next) {
        *work += next.len();
        if Arc::strong_count(targets) > 1 {
            *work += targets.len();
        }
        Arc::make_mut(targets).extend(next.iter().copied());
    }
}

/// Iterative Kosaraju; consumer → dependency DFS finishes dependencies first.
pub(crate) fn components(dependencies: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let mut seen = HashSet::new();
    let mut finished = Vec::new();
    for node in 0..dependencies.len() {
        let mut stack = vec![(node, false)];
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                finished.push(node);
            } else if seen.insert(node) {
                stack.push((node, true));
                stack.extend(dependencies[node].iter().rev().map(|&next| (next, false)));
            }
        }
    }
    let mut reverse = vec![Vec::new(); dependencies.len()];
    for (node, targets) in dependencies.iter().enumerate() {
        for &target in targets {
            reverse[target].push(node);
        }
    }
    seen.clear();
    let mut result = Vec::new();
    for &node in finished.iter().rev() {
        let mut component = Vec::new();
        let mut stack = vec![node];
        while let Some(node) = stack.pop() {
            if seen.insert(node) {
                component.push(node);
                stack.extend(&reverse[node]);
            }
        }
        if !component.is_empty() {
            component.sort_unstable();
            result.push(component);
        }
    }
    result.reverse();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::input;

    #[test]
    fn shared_connector_projections_cache_empty_chains_dense_dags_and_cycles() {
        for dense in [false, true] {
            for cycle in [false, true] {
                for nonempty in [false, true] {
                    let count = if dense { 100 } else { 1000 };
                    let connectors: Vec<_> = (0..count).map(GraphNode::Module).collect();
                    let nodes: Vec<_> = (0..1000)
                        .map(|i| GraphNode::Segment(SegmentKey::new(count, i)))
                        .collect();
                    let effectful = nodes.iter().copied().collect();
                    let mut graph = Dependencies::new();
                    for &node in &nodes {
                        graph.insert(node, vec![connectors[0]]);
                    }
                    for (i, &node) in connectors.iter().enumerate() {
                        let next = if dense {
                            connectors[i + 1..].to_vec()
                        } else {
                            connectors.get(i + 1).copied().into_iter().collect()
                        };
                        graph.insert(node, next);
                    }
                    if cycle {
                        graph
                            .get_mut(connectors.last().unwrap())
                            .unwrap()
                            .push(connectors[0]);
                    }
                    if nonempty {
                        graph
                            .get_mut(connectors.last().unwrap())
                            .unwrap()
                            .push(nodes[0]);
                    }
                    let (projected, work) = project_all(&graph, &nodes, &effectful);
                    let expected = if nonempty {
                        HashSet::from([nodes[0]])
                    } else {
                        HashSet::new()
                    };
                    assert!(projected.iter().all(|targets| **targets == expected));
                    let links: usize = graph.values().map(Vec::len).sum();
                    assert!(
                        work <= 8 * (links + nodes.len()),
                        "{work} visits for {links} links"
                    );
                    eprintln!("dense={dense}, cycle={cycle}, nonempty={nonempty}: {work} visits, {links} links");
                    let reversed: Dependencies = graph
                        .into_iter()
                        .map(|(node, mut next)| {
                            next.reverse();
                            (node, next)
                        })
                        .collect();
                    assert_eq!(projected, project_all(&reversed, &nodes, &effectful).0);
                }
            }
        }
    }

    #[test]
    fn long_connector_chain_shares_broad_first_effect_projection() {
        let nodes: Vec<_> = (0..1000)
            .map(|i| GraphNode::Segment(SegmentKey::new(0, i)))
            .collect();
        let effectful = nodes.iter().copied().collect();
        let mut graph = Dependencies::new();
        for file in 0..20_000 {
            graph.insert(GraphNode::Module(file), vec![GraphNode::Module(file + 1)]);
        }
        graph.insert(GraphNode::Module(20_000), nodes.clone());
        for &node in &nodes {
            graph.insert(node, vec![GraphNode::Module(0)]);
        }
        let (projected, work) = project_all(&graph, &nodes, &effectful);
        assert_eq!(*projected[0], effectful);
        assert!(projected
            .iter()
            .all(|targets| Arc::ptr_eq(targets, &projected[0])));
        assert!(work < 150_000, "{work} visits");
    }

    #[test]
    fn connector_graph_is_linear_before_effect_projection() {
        let source = format!(
            "{}{}",
            "import 'dep.mjs';".repeat(500),
            "run();".repeat(500)
        );
        let graph =
            SourceGraph::from_owned([input("entry.mjs", &source), input("dep.mjs", "dep();")]);
        let effects = (500..1000)
            .map(|index| GraphNode::Segment(SegmentKey::new(0, index)))
            .chain([GraphNode::Segment(SegmentKey::new(1, 0))])
            .collect();
        let mut retention = HashSet::new();
        let ordering = module_dependencies(
            &graph,
            &BTreeSet::from([0, 1]),
            &[0],
            &effects,
            &mut retention,
        );
        let connector_edges = ordering.values().map(Vec::len).sum::<usize>();
        assert!(connector_edges <= 2002, "{connector_edges} connector edges");
        assert_eq!(retention.len(), 501);
        let plan = crate::retain(&graph, &[0], &[]).unwrap();
        assert_eq!(plan.effect_order[0], SegmentKey::new(1, 0));
        assert_eq!(plan.effect_order.len(), 501);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        assert!(plan.edges.len() <= 2500, "{} edges", plan.edges.len());
    }
}
