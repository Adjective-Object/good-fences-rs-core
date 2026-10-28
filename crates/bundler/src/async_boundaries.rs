//! Build-local extraction of private, capture-free function declaration islands.
use std::collections::{BTreeMap, BTreeSet};

use ast_segmenter::views::{DeferredForm, OperationKind};
use eyre::OptionExt;
use source_graph::{EdgeKind, ExportIdentity, GraphNode, SegmentKey, SourceGraph};

use crate::chunks::{
    add_reachability_roots, compact_reachability, node_id, AsyncSiteId, ChunkId, ChunkPlan,
    GeneratedBoundary, RootId, RootSeed,
};
use crate::{policy::EffectPolicy, RetentionEdgeKind};

pub(crate) fn validate(graph: &SourceGraph, plan: &ChunkPlan) -> eyre::Result<()> {
    for (site, boundary) in &plan.generated_boundaries {
        let edge = graph
            .edges()
            .get(boundary.source_edge)
            .ok_or_eyre("missing generated source edge")?;
        let GraphNode::Segment(from) = edge.from else {
            eyre::bail!("generated consumer is not a segment")
        };
        let facts = graph
            .file_facts(from.file_id)
            .ok_or_eyre("missing generated consumer facts")?;
        let reference = facts
            .references()
            .get(boundary.reference as usize)
            .ok_or_eyre("missing generated reference")?;
        eyre::ensure!(
            edge.kind == EdgeKind::LocalReference
                && edge.to == GraphNode::Segment(boundary.target)
                && edge.async_deferred
                && reference.async_deferred()
                && edge.read
                && !edge.write
                && !edge.type_only
                && reference.read()
                && !reference.write()
                && !reference.type_only()
                && reference.span() == edge.site
                && reference.binding() == edge.binding
                && graph.file_path(from.file_id) == Some(site.module.as_path())
                && site.offset == reference.span().start
                && site.candidate.is_none(),
            "invalid generated reference occurrence"
        );
        let seed = plan
            .roots
            .get(&RootId::Async(site.clone()))
            .ok_or_eyre("missing generated root")?;
        eyre::ensure!(
            seed.evaluation_roots.is_empty()
                && seed.binding_roots == [GraphNode::Segment(boundary.target)]
                && !plan
                    .retention
                    .edges
                    .iter()
                    .any(|e| e.source_edge == Some(boundary.source_edge)),
            "generated reference must replace synchronous demand"
        );
    }
    Ok(())
}

pub(crate) fn extract(
    graph: &SourceGraph,
    original: &ChunkPlan,
    policy: &EffectPolicy,
) -> eyre::Result<ChunkPlan> {
    eyre::ensure!(
        original.is_current(graph, policy),
        "stale async extraction plan"
    );
    if !original.generated_boundaries.is_empty() {
        validate(graph, original)?;
        return Ok(original.clone());
    }
    // Unknown external surfaces and evaluation must retain their native seams.
    if graph
        .edges()
        .iter()
        .any(|e| matches!(e.to, GraphNode::Unlinked { .. }))
    {
        return Ok(original.clone());
    }
    let symbols = crate::symbols::Symbols::link(graph, original)?;
    let live: BTreeSet<_> = original.retention.segments.iter().map(|s| s.key).collect();
    let mut candidates = BTreeSet::new();
    for &key in &live {
        let facts = graph
            .file_facts(key.file_id)
            .ok_or_eyre("missing extraction facts")?;
        let recipe = facts
            .emission()
            .segments()
            .get(key.segment_idx as usize)
            .unwrap();
        if facts.commonjs()
            || !facts.top_level_await().is_empty()
            || facts.dependencies().iter().any(|d| !d.type_only())
            || !matches!(recipe.deferred(), DeferredForm::Function)
            || recipe.placement().declarations().len() != 1
            || !recipe.placement().instantiable()
        {
            continue;
        }
        let binding = recipe.placement().declarations().get(0).unwrap();
        let identity = ExportIdentity::Binding(key.file_id, binding);
        if facts
            .bindings()
            .iter()
            .any(|b| b.id() == binding && b.declarations().len() != 1)
            || symbols
                .exports
                .values()
                .any(|exports| exports.values().any(|s| s.identity == identity))
            || original
                .roots
                .values()
                .any(|root| root.binding_roots.contains(&GraphNode::Segment(key)))
        {
            continue;
        }
        candidates.insert(key);
    }
    let declaration = |file, binding| {
        let facts = graph.file_facts(file)?;
        let b = facts
            .bindings()
            .iter()
            .find(|b| b.id() == binding && b.top_level())?;
        let index = graph
            .file_segments(file)?
            .partition_point(|s| s.span.end <= b.declarations().get(0).unwrap().start);
        Some(SegmentKey::new(file, index as u32))
    };
    // Removing an unsupported member may make a former internal reference an
    // incoming synchronous use. Iterate to a fixed point, without a call graph.
    loop {
        let mut rejected = BTreeSet::new();
        for &key in &candidates {
            let facts = graph.file_facts(key.file_id).unwrap();
            let span = graph.file_segments(key.file_id).unwrap()[key.segment_idx as usize].span;
            for reference in facts
                .references()
                .iter()
                .filter(|r| !r.type_only() && crate::link::contains(span, r.span()))
            {
                if let Some(target) = reference
                    .binding()
                    .and_then(|b| declaration(key.file_id, b))
                {
                    if !candidates.contains(&target) || reference.write() {
                        rejected.insert(key);
                    }
                }
            }
            for edge in &original.retention.edges {
                if edge.to != GraphNode::Segment(key) {
                    continue;
                }
                let GraphNode::Segment(from) = edge.from else {
                    rejected.insert(key);
                    continue;
                };
                if candidates.contains(&from) {
                    continue;
                }
                let Some(index) = edge.source_edge else {
                    rejected.insert(key);
                    continue;
                };
                let source = &graph.edges()[index];
                let consumer = graph.file_facts(from.file_id).unwrap();
                let reference = consumer
                    .references()
                    .iter()
                    .position(|r| r.span() == source.site);
                if edge.kind != RetentionEdgeKind::Binding
                    || source.kind != EdgeKind::LocalReference
                    || from.file_id != key.file_id
                    || !source.async_deferred || !source.read || source.write
                    || reference.is_none_or(|reference| {
                        !consumer.emission().segments().get(from.segment_idx as usize).unwrap().operations().iter().any(|op|
                            matches!(op.kind(), OperationKind::Reference { reference: r, .. } if r as usize == reference))
                    })
                    || consumer.invocations().iter().any(|invocation|
                        (invocation.optional_call() || invocation.optional_chain())
                        && crate::link::contains(invocation.span(), source.site))
                    || original.retention.effects.iter().any(|effects| effects.key == from
                        && effects.summary.rewrites.iter().any(|rewrite|
                            crate::link::contains(rewrite.invocation, source.site)))
                {
                    rejected.insert(key);
                }
            }
        }
        if rejected.is_empty() {
            break;
        }
        candidates.retain(|key| !rejected.contains(key));
    }
    let mut plan = original.clone();
    for edge in &original.retention.edges {
        let (GraphNode::Segment(from), GraphNode::Segment(target)) = (edge.from, edge.to) else {
            continue;
        };
        if !candidates.contains(&target) || candidates.contains(&from) {
            continue;
        }
        let index = edge
            .source_edge
            .ok_or_eyre("generated boundary needs checked occurrence")?;
        let source = &graph.edges()[index];
        let reference = graph
            .file_facts(from.file_id)
            .unwrap()
            .references()
            .iter()
            .position(|r| r.span() == source.site)
            .ok_or_eyre("missing boundary reference")? as u32;
        let site = AsyncSiteId {
            module: graph.file_path(from.file_id).unwrap().to_owned(),
            offset: source.site.start,
            candidate: None,
        };
        eyre::ensure!(
            !plan.roots.contains_key(&RootId::Async(site.clone())),
            "conflicting generated reference root"
        );
        plan.roots.insert(
            RootId::Async(site.clone()),
            RootSeed {
                id: RootId::Async(site.clone()),
                evaluation_roots: vec![],
                binding_roots: vec![GraphNode::Segment(target)],
            },
        );
        plan.generated_boundaries.insert(
            site,
            GeneratedBoundary {
                source_edge: index,
                reference,
                target,
            },
        );
    }
    let replaced: BTreeSet<_> = plan
        .generated_boundaries
        .values()
        .map(|b| b.source_edge)
        .collect();
    plan.retention
        .edges
        .retain(|e| !e.source_edge.is_some_and(|i| replaced.contains(&i)));
    plan.retention.edges.retain(|e| {
        !(e.kind == RetentionEdgeKind::BindingEvaluation
            && matches!(e.from, GraphNode::Segment(key) if candidates.contains(&key)))
    });
    plan.reachability = compact_reachability(
        graph,
        &plan.retention,
        policy,
        &plan.roots.values().cloned().collect::<Vec<_>>(),
    )?;
    let empty = crate::chunks::RootMask::default();
    for mask in plan.node_roots.values_mut() {
        *mask = empty.clone();
    }
    add_reachability_roots(&plan.reachability, &mut plan.node_roots);
    plan.chunks = BTreeMap::new();
    for (node, roots) in &plan.node_roots {
        if !roots.is_empty() {
            plan.chunks
                .entry(ChunkId(roots.clone()))
                .or_default()
                .push(node.clone());
        }
    }
    // Every extracted declaration must still be reached through a real use.
    for &key in &candidates {
        eyre::ensure!(
            !plan.node_roots[&node_id(graph, GraphNode::Segment(key))?].is_empty(),
            "unreachable extracted declaration"
        );
    }
    validate(graph, &plan)?;
    Ok(plan)
}
