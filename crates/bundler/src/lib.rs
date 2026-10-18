//! Retention, exact chunks and native ESM emission, independent of usage reports.
pub mod build;
pub mod chunks;
mod cjs;
pub mod effects;
pub mod emit;
pub mod link;
pub mod manifest;
pub mod policy;
pub mod publish;
pub mod rewrite;

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use ast_segmenter::owned::ModuleFacts;
use eyre::{ensure, OptionExt};
use source_graph::{
    Activation, DependencyTarget, EdgeKind, EdgePurpose, GraphNode, GraphRevision, SegmentKey,
    SourceGraph,
};

#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod pure_tests;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetentionReason {
    BindingRoot,
    TopLevelEffect,
    Dependency,
    NamespaceConservative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedSegment {
    pub key: SegmentKey,
    pub reasons: Vec<RetentionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentEffects {
    pub key: SegmentKey,
    pub summary: effects::StatementEffects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RetentionEdgeKind {
    StaticEvaluation,
    BindingEvaluation,
    ModuleEffect,
    EffectOrder,
    Binding,
    Require,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RetentionEdge {
    pub from: GraphNode,
    pub to: GraphNode,
    pub kind: RetentionEdgeKind,
    /// Index in SourceGraph::edges at this plan's graph revision.
    pub source_edge: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPlan {
    pub revision: GraphRevision,
    pub policy_fingerprint: [u8; 32],
    /// Required evaluation bodies, not an eager-execution schedule.
    pub modules: Vec<u32>,
    pub segments: Vec<RetainedSegment>,
    pub edges: Vec<RetentionEdge>,
    /// Retained async occurrences, not traversed until P5 assigns boundary roots.
    pub async_boundaries: Vec<usize>,
    /// All evaluated-module statements, including recipes for otherwise dead ones.
    pub effects: Vec<SegmentEffects>,
}

impl RetentionPlan {
    pub fn is_current(&self, graph: &SourceGraph, policy: &policy::EffectPolicy) -> bool {
        self.revision == graph.revision() && self.policy_fingerprint == policy.fingerprint()
    }
}

/// Binding roots must include the checked export route when requesting an export.
/// Namespace module roots conservatively retain all target statements.
pub fn retain(
    graph: &SourceGraph,
    evaluation_roots: &[u32],
    binding_roots: &[GraphNode],
) -> eyre::Result<RetentionPlan> {
    retain_with_policy(
        graph,
        evaluation_roots,
        binding_roots,
        &policy::EffectPolicy::default(),
    )
}

pub fn retain_with_policy(
    graph: &SourceGraph,
    evaluation_roots: &[u32],
    binding_roots: &[GraphNode],
    policy: &policy::EffectPolicy,
) -> eyre::Result<RetentionPlan> {
    graph.validate_links()?;
    let mut analysis = Retention {
        graph,
        policy,
        modules: BTreeSet::new(),
        namespaces: HashSet::new(),
        segments: BTreeMap::new(),
        edges: HashSet::new(),
        async_boundaries: BTreeSet::new(),
        queue: VecDeque::new(),
        effects: BTreeMap::new(),
        values: HashSet::new(),
    };
    for &file in evaluation_roots {
        analysis.queue.push_back(Demand::Evaluation(file));
    }
    for &node in binding_roots {
        analysis
            .queue
            .push_back(Demand::Binding(node, RetentionReason::BindingRoot));
    }
    while let Some(demand) = analysis.queue.pop_front() {
        match demand {
            Demand::Evaluation(file) => analysis.evaluate(file)?,
            Demand::Binding(node, reason) => analysis.binding(node, reason)?,
        }
    }
    let mut previous = BTreeMap::new();
    for (&key, summary) in &analysis.effects {
        if !summary.is_effectful() {
            continue;
        }
        let node = GraphNode::Segment(key);
        analysis.edges.insert(RetentionEdge {
            from: GraphNode::Module(key.file_id),
            to: node,
            kind: RetentionEdgeKind::ModuleEffect,
            source_edge: None,
        });
        if let Some(previous) = previous.insert(key.file_id, node) {
            analysis.edges.insert(RetentionEdge {
                from: node,
                to: previous,
                kind: RetentionEdgeKind::EffectOrder,
                source_edge: None,
            });
        }
    }
    let mut edges: Vec<_> = analysis.edges.into_iter().collect();
    edges.sort_by_key(|e| (node_key(e.from), node_key(e.to), e.kind, e.source_edge));
    Ok(RetentionPlan {
        revision: graph.revision(),
        policy_fingerprint: policy.fingerprint(),
        modules: analysis.modules.into_iter().collect(),
        segments: analysis
            .segments
            .into_iter()
            .map(|(key, reasons)| RetainedSegment {
                key,
                reasons: reasons.into_iter().collect(),
            })
            .collect(),
        edges,
        async_boundaries: analysis.async_boundaries.into_iter().collect(),
        effects: analysis
            .effects
            .into_iter()
            .map(|(key, summary)| SegmentEffects { key, summary })
            .collect(),
    })
}

fn node_key(node: GraphNode) -> (u32, u32, u32) {
    match node {
        GraphNode::Module(file) => (file, 0, 0),
        GraphNode::Segment(key) => (key.file_id, 1, key.segment_idx),
        GraphNode::Unlinked {
            file_id,
            dependency,
        } => (file_id, 2, dependency.0),
    }
}

enum Demand {
    Evaluation(u32),
    Binding(GraphNode, RetentionReason),
}

struct Retention<'g, 'p> {
    graph: &'g SourceGraph,
    policy: &'p policy::EffectPolicy,
    modules: BTreeSet<u32>,
    namespaces: HashSet<u32>,
    segments: BTreeMap<SegmentKey, BTreeSet<RetentionReason>>,
    edges: HashSet<RetentionEdge>,
    async_boundaries: BTreeSet<usize>,
    queue: VecDeque<Demand>,
    effects: BTreeMap<SegmentKey, effects::StatementEffects>,
    values: HashSet<SegmentKey>,
}

impl Retention<'_, '_> {
    fn facts(&self, file: u32) -> eyre::Result<&ModuleFacts> {
        self.graph
            .file_facts(file)
            .ok_or_eyre("bundler requires live modules with owned source facts")
    }

    fn evaluate(&mut self, file: u32) -> eyre::Result<()> {
        self.facts(file)?;
        if !self.modules.insert(file) {
            return Ok(());
        }
        let graph = self.graph;
        let statements = graph
            .file_segments(file)
            .ok_or_eyre("missing module statements")?;
        for (index, statement) in statements.iter().enumerate() {
            let key = SegmentKey::new(file, index as u32);
            let summary = effects::summarize_with_policy(
                self.facts(file)?,
                statement.span,
                self.values.contains(&key),
                self.policy,
            );
            if summary.is_effectful() {
                let node = GraphNode::Segment(key);
                self.queue
                    .push_back(Demand::Binding(node, RetentionReason::TopLevelEffect));
            }
            self.effects.insert(key, summary);
            for (index, edge) in graph.outgoing_indexed(GraphNode::Segment(key)) {
                if matches!(edge.kind, EdgeKind::Import | EdgeKind::ReExport)
                    && edge.purpose != EdgePurpose::Binding
                    && !edge.type_only
                {
                    self.edges.insert(RetentionEdge {
                        from: GraphNode::Module(file),
                        to: edge.to,
                        kind: RetentionEdgeKind::StaticEvaluation,
                        source_edge: Some(index),
                    });
                    self.target(edge.to, false)?;
                }
            }
        }
        Ok(())
    }

    fn target(&mut self, node: GraphNode, binding: bool) -> eyre::Result<()> {
        match node {
            GraphNode::Module(file) if !binding => self.queue.push_back(Demand::Evaluation(file)),
            GraphNode::Unlinked {
                file_id,
                dependency,
            } => {
                ensure!(
                    matches!(
                        self.graph.dependency_target(file_id, dependency),
                        Some(DependencyTarget::External(_))
                    ),
                    "unresolved retained dependency"
                );
            }
            _ => self
                .queue
                .push_back(Demand::Binding(node, RetentionReason::Dependency)),
        }
        Ok(())
    }

    fn binding(&mut self, node: GraphNode, reason: RetentionReason) -> eyre::Result<()> {
        let key = match node {
            GraphNode::Segment(key) => key,
            GraphNode::Module(file) => {
                self.facts(file)?;
                self.queue.push_back(Demand::Evaluation(file));
                if self.namespaces.insert(file) {
                    for index in 0..self
                        .graph
                        .file_segments(file)
                        .ok_or_eyre("missing namespace statements")?
                        .len()
                    {
                        let key = SegmentKey::new(file, index as u32);
                        self.queue.push_back(Demand::Binding(
                            GraphNode::Segment(key),
                            RetentionReason::NamespaceConservative,
                        ));
                    }
                }
                return Ok(());
            }
            GraphNode::Unlinked { .. } => {
                eyre::bail!("an unlinked dependency cannot be a binding root");
            }
        };
        self.facts(key.file_id)?;
        ensure!(
            self.graph
                .file_segments(key.file_id)
                .is_some_and(|s| (key.segment_idx as usize) < s.len()),
            "invalid binding segment root"
        );
        let reasons = self.segments.entry(key).or_default();
        let first = reasons.is_empty();
        reasons.insert(reason);
        if reason != RetentionReason::TopLevelEffect && self.values.insert(key) {
            let span = self
                .graph
                .file_segments(key.file_id)
                .ok_or_eyre("missing binding statements")?[key.segment_idx as usize]
                .span;
            let summary =
                effects::summarize_with_policy(self.facts(key.file_id)?, span, true, self.policy);
            self.effects.insert(key, summary);
        }
        if !first {
            return Ok(());
        }
        self.edges.insert(RetentionEdge {
            from: node,
            to: GraphNode::Module(key.file_id),
            kind: RetentionEdgeKind::BindingEvaluation,
            source_edge: None,
        });
        self.queue.push_back(Demand::Evaluation(key.file_id));
        let graph = self.graph;
        for (index, edge) in graph.outgoing_indexed(node).filter(|(_, e)| !e.type_only) {
            if edge.activation == Activation::Asynchronous {
                self.async_boundaries.insert(index);
                continue;
            }
            if edge.purpose == EdgePurpose::Binding || edge.kind == EdgeKind::LocalReference {
                self.edges.insert(RetentionEdge {
                    from: node,
                    to: edge.to,
                    kind: RetentionEdgeKind::Binding,
                    source_edge: Some(index),
                });
                self.target(edge.to, true)?;
            } else if edge.kind == EdgeKind::Require {
                self.edges.insert(RetentionEdge {
                    from: node,
                    to: edge.to,
                    kind: RetentionEdgeKind::Require,
                    source_edge: Some(index),
                });
                // Opaque require consumers can inspect any namespace property.
                self.target(edge.to, true)?;
            }
        }
        Ok(())
    }
}
