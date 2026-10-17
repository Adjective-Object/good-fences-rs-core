//! Exact synchronous reachability over retention constraints, not execution order.
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::PathBuf;

use eyre::{ensure, OptionExt};
use source_graph::{Activation, GraphNode, GraphRevision, SourceGraph};

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
    Segment {
        module: PathBuf,
        index: u32,
    },
    External {
        module: PathBuf,
        offset: u32,
    },
    /// None initializes the proxy/default; Some is one requested named snapshot.
    CommonJsProxy {
        module: PathBuf,
        name: Option<String>,
    },
    CommonJsNamespace(PathBuf),
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
        GraphNode::Unlinked { dependency, .. } => {
            ensure!(
                matches!(
                    graph.dependency_target(file, dependency),
                    Some(source_graph::DependencyTarget::External(_))
                ),
                "chunk node is not a resolved external site"
            );
            NodeId::External {
                module,
                offset: dependency.0,
            }
        }
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
        NodeId::External { .. } | NodeId::CommonJsProxy { .. } | NodeId::CommonJsNamespace(_) => {
            eyre::bail!("async consumer must be a source node")
        }
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
                if matches!(
                    edge.kind,
                    RetentionEdgeKind::Binding | RetentionEdgeKind::Require
                ) {
                    enqueue_binding(retention, edge.to, &mut queue);
                } else {
                    queue.push_back(edge.to);
                }
            }
        }
        for file in retention
            .modules
            .iter()
            .copied()
            .filter(|file| graph.file_facts(*file).is_some_and(|facts| facts.commonjs))
        {
            let module = graph.file_path(file).unwrap().to_owned();
            let mut demanded = BTreeSet::new();
            let direct_entry = root.evaluation_roots.contains(&file)
                || root.binding_roots.iter().any(|node| match node {
                    GraphNode::Module(target) => *target == file,
                    GraphNode::Segment(key) => key.file_id == file,
                    GraphNode::Unlinked { .. } => false,
                });
            let mut initialize = direct_entry;
            let mut namespace = direct_entry;
            for (from, request) in crate::cjs::requests(graph, retention, file) {
                if !reached.contains(&node_id(graph, from)?) {
                    continue;
                }
                initialize = true;
                match request {
                    ast_segmenter::owned::ImportedName::Name(name) => {
                        demanded.insert(name);
                    }
                    ast_segmenter::owned::ImportedName::Namespace => namespace = true,
                }
            }
            if initialize {
                reached.insert(NodeId::CommonJsProxy {
                    module: module.clone(),
                    name: None,
                });
                reached.extend(
                    demanded
                        .into_iter()
                        .filter(|name| name != "default")
                        .map(|name| NodeId::CommonJsProxy {
                            module: module.clone(),
                            name: Some(name),
                        }),
                );
                if namespace {
                    reached.insert(NodeId::CommonJsNamespace(module.clone()));
                }
            }
        }
        result.insert(root.id.clone(), reached);
    }
    Ok(result)
}

pub(crate) fn enqueue_binding(
    retention: &RetentionPlan,
    node: GraphNode,
    queue: &mut VecDeque<GraphNode>,
) {
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

/// The exact nonempty root set is the logical identity, not a filename/ordinal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId(pub BTreeSet<RootId>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    pub revision: GraphRevision,
    pub policy_fingerprint: [u8; 32],
    pub retention: RetentionPlan,
    /// Graph-local seeds, retained for facade construction at this revision.
    pub roots: BTreeMap<RootId, RootSeed>,
    pub reachability: BTreeMap<RootId, BTreeSet<NodeId>>,
    /// Includes dead source modules/segments with empty sets.
    pub node_roots: BTreeMap<NodeId, BTreeSet<RootId>>,
    /// Exactly one chunk per nonempty set, without merging or hoisting.
    pub chunks: BTreeMap<ChunkId, Vec<NodeId>>,
}

impl ChunkPlan {
    pub fn is_current(&self, graph: &SourceGraph, policy: &EffectPolicy) -> bool {
        self.revision == graph.revision()
            && self.policy_fingerprint == policy.fingerprint()
            && self.retention.is_current(graph, policy)
    }

    pub fn chunk_for(&self, node: &NodeId) -> Option<ChunkId> {
        self.node_roots
            .get(node)
            .filter(|roots| !roots.is_empty())
            .cloned()
            .map(ChunkId)
    }
}

/// Activate only reachable async occurrences, recomputing P4 retention as new
/// namespace demands appear. No parent root crosses an asynchronous edge.
pub fn partition(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
) -> eyre::Result<ChunkPlan> {
    let mut roots = BTreeMap::new();
    for entry in entries {
        ensure!(
            matches!(entry.id, RootId::Entry(_)),
            "initial roots must be entries"
        );
        ensure!(
            !entry.evaluation_roots.is_empty() || !entry.binding_roots.is_empty(),
            "entry requires an evaluation or binding root"
        );
        ensure!(
            !entry
                .binding_roots
                .iter()
                .any(|n| matches!(n, GraphNode::Unlinked { .. })),
            "entry cannot bind an external site"
        );
        ensure!(
            roots.insert(entry.id.clone(), entry.clone()).is_none(),
            "duplicate entry identity"
        );
    }
    let (retention, reachability) = loop {
        let evaluation: Vec<_> = roots
            .values()
            .flat_map(|r| r.evaluation_roots.iter().copied())
            .collect();
        let bindings: Vec<_> = roots
            .values()
            .flat_map(|r| r.binding_roots.iter().copied())
            .filter(|n| !matches!(n, GraphNode::Unlinked { .. }))
            .collect();
        let retention = crate::retain_with_policy(graph, &evaluation, &bindings, policy)?;
        let seeds: Vec<_> = roots.values().cloned().collect();
        let reached = reference_reachability(graph, &retention, policy, &seeds)?;
        let mut added = false;
        for &index in &retention.async_boundaries {
            let consumer = node_id(graph, graph.edges()[index].from)?;
            if !reached.values().any(|nodes| nodes.contains(&consumer)) {
                continue;
            }
            let seed = async_seed(graph, index)?;
            if let Some(existing) = roots.get(&seed.id) {
                ensure!(*existing == seed, "conflicting async site identity");
            } else {
                roots.insert(seed.id.clone(), seed);
                added = true;
            }
        }
        if !added {
            break (retention, reached);
        }
    };
    let mut node_roots: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for file in graph.module_ids() {
        if let Some(segments) = graph.file_segments(file) {
            node_roots
                .entry(node_id(graph, GraphNode::Module(file))?)
                .or_default();
            for index in 0..segments.len() {
                node_roots
                    .entry(node_id(
                        graph,
                        GraphNode::Segment(source_graph::SegmentKey::new(file, index as u32)),
                    )?)
                    .or_default();
            }
        }
    }
    for (root, nodes) in &reachability {
        for node in nodes {
            node_roots
                .entry(node.clone())
                .or_default()
                .insert(root.clone());
        }
    }
    let mut chunks: BTreeMap<ChunkId, Vec<NodeId>> = BTreeMap::new();
    for (node, roots) in &node_roots {
        if !roots.is_empty() {
            chunks
                .entry(ChunkId(roots.clone()))
                .or_default()
                .push(node.clone());
        }
    }
    Ok(ChunkPlan {
        revision: graph.revision(),
        policy_fingerprint: policy.fingerprint(),
        retention,
        roots,
        reachability,
        node_roots,
        chunks,
    })
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
