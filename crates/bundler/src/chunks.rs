//! Exact synchronous reachability over retention constraints, not execution order.
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::PathBuf;

use eyre::{ensure, OptionExt};
use source_graph::{Activation, GraphNode, SourceGraph};

use crate::{policy::EffectPolicy, RetentionEdgeKind, RetentionPlan};

/// Source occurrence identity, independent of file/edge insertion order.
/// Offsets identify sites only within the stamped source graph revision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AsyncSiteId {
    pub module: PathBuf,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootId {
    Entry(String),
    Async(AsyncSiteId),
}

/// Canonical diagnostic identity; graph-local numeric IDs never name chunks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeId {
    Module(PathBuf),
    Segment { module: PathBuf, index: u32 },
    External { module: PathBuf, offset: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSeed {
    pub id: RootId,
    pub evaluation_roots: Vec<u32>,
    /// A module binding root requests its namespace, unlike evaluation alone.
    pub binding_roots: Vec<GraphNode>,
}

pub fn node_id(graph: &SourceGraph, node: GraphNode) -> eyre::Result<NodeId> {
    let file = match node {
        GraphNode::Module(file) => file,
        GraphNode::Segment(key) => key.file_id,
        GraphNode::Unlinked { file_id, .. } => file_id,
    };
    let module = graph
        .file_path(file)
        .ok_or_eyre("missing chunk node module")?
        .to_owned();
    Ok(match node {
        GraphNode::Module(_) => NodeId::Module(module),
        GraphNode::Segment(key) => {
            ensure!(
                graph
                    .file_segments(file)
                    .is_some_and(|s| (key.segment_idx as usize) < s.len()),
                "missing chunk segment"
            );
            NodeId::Segment {
                module,
                index: key.segment_idx,
            }
        }
        GraphNode::Unlinked { dependency, .. } => NodeId::External {
            module,
            offset: dependency.0,
        },
    })
}

pub fn async_seed(graph: &SourceGraph, edge_index: usize) -> eyre::Result<RootSeed> {
    let edge = graph
        .edges()
        .get(edge_index)
        .ok_or_eyre("missing async occurrence")?;
    ensure!(
        edge.activation == Activation::Asynchronous && !edge.type_only,
        "not a runtime async occurrence"
    );
    let dependency = edge
        .dependency
        .ok_or_eyre("async occurrence has no site identity")?;
    let module = match node_id(graph, edge.from)? {
        NodeId::Module(path) | NodeId::Segment { module: path, .. } => path,
        NodeId::External { .. } => eyre::bail!("async consumer cannot be external"),
    };
    Ok(RootSeed {
        id: RootId::Async(AsyncSiteId {
            module,
            offset: dependency.0,
        }),
        evaluation_roots: vec![],
        binding_roots: vec![edge.to],
    })
}

/// Deliberately simple reference oracle: one independent flood fill per root.
/// Deferred synchronous dependencies supply code, not permission to run it.
pub fn reference_reachability(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    policy: &EffectPolicy,
    roots: &[RootSeed],
) -> eyre::Result<BTreeMap<RootId, BTreeSet<NodeId>>> {
    ensure!(
        retention.is_current(graph, policy),
        "stale retention revision or effect policy"
    );
    let mut result = BTreeMap::new();
    for root in roots {
        ensure!(!result.contains_key(&root.id), "duplicate root identity");
        let mut queue: VecDeque<_> = root
            .evaluation_roots
            .iter()
            .copied()
            .map(GraphNode::Module)
            .collect();
        for &node in &root.binding_roots {
            enqueue_binding(retention, node, &mut queue);
        }
        let mut visited = HashSet::new();
        let mut reached = BTreeSet::new();
        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }
            reached.insert(node_id(graph, node)?);
            ensure!(
                match node {
                    GraphNode::Module(file) => retention.modules.contains(&file),
                    GraphNode::Segment(key) => retention.segments.iter().any(|s| s.key == key),
                    GraphNode::Unlinked { .. } => true,
                },
                "root or dependency is absent from retention"
            );
            for edge in retention.edges.iter().filter(|e| e.from == node) {
                if let Some(index) = edge.source_edge {
                    let source = graph
                        .edges()
                        .get(index)
                        .ok_or_eyre("missing retained occurrence")?;
                    if source.activation == Activation::Asynchronous || source.type_only {
                        continue;
                    }
                }
                if edge.kind == RetentionEdgeKind::Binding {
                    enqueue_binding(retention, edge.to, &mut queue);
                } else {
                    queue.push_back(edge.to);
                }
            }
        }
        result.insert(root.id.clone(), reached);
    }
    Ok(result)
}

fn enqueue_binding(retention: &RetentionPlan, node: GraphNode, queue: &mut VecDeque<GraphNode>) {
    queue.push_back(node);
    if let GraphNode::Module(file) = node {
        // Namespace demand is conservative in P4; evaluation demand is not.
        queue.extend(
            retention
                .segments
                .iter()
                .filter(|s| s.key.file_id == file)
                .map(|s| GraphNode::Segment(s.key)),
        );
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
