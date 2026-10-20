//! Retention, exact chunks and native ESM emission, independent of usage reports.
mod assembly;
pub mod build;
pub mod chunks;
mod cjs;
mod context;
mod css;
mod css_loading;
mod css_names;
mod css_selection;
mod css_syntax;
mod effect_analysis;
mod effect_dependencies;
pub mod effects;
pub mod emit;
pub mod execution;
pub mod link;
pub mod manifest;
mod output_url;
pub mod policy;
pub mod publish;
mod resource;
pub mod rewrite;
mod symbols;

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
    /// Native external export checks survive even when a binding is unused.
    NativeValidation,
    BindingEvaluation,
    ModuleEffect,
    /// Scheduling only; never traverse for binding demand or code availability.
    EffectOrder,
    Binding,
    /// Export routes traversed only by namespace binding demand, not evaluation.
    Namespace,
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
    /// Dependency-first order of available effects, not permission to activate them.
    /// Independent of binding/availability edges and deferred/async activation.
    /// Native module jobs, not this scalar partial order, schedule async evaluation.
    pub effect_order: Vec<SegmentKey>,
    /// Deterministic effect-order cycle diagnostics; never binding/TDZ exemptions.
    pub warnings: Vec<String>,
}

impl RetentionPlan {
    pub fn is_current(&self, graph: &SourceGraph, policy: &policy::EffectPolicy) -> bool {
        self.revision == graph.revision() && self.policy_fingerprint == policy.fingerprint()
    }
}

/// Binding roots must include the checked export route when requesting an export.
/// Namespace module roots retain visible export routes and ordinary effects.
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
    let ecco_effects = effect_analysis::analyze(graph, policy)?;
    retain_with_effects(
        graph,
        evaluation_roots,
        binding_roots,
        policy,
        &ecco_effects,
    )
}

fn retain_with_effects(
    graph: &SourceGraph,
    evaluation_roots: &[u32],
    binding_roots: &[GraphNode],
    policy: &policy::EffectPolicy,
    ecco_effects: &BTreeMap<SegmentKey, bool>,
) -> eyre::Result<RetentionPlan> {
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
        export_names: link::namespace_export_names(graph, |_| BTreeSet::from(["default".into()])),
        ecco_effects,
        native_externals: graph
            .module_ids()
            .any(|file| link::has_externals(graph, file)),
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
    let (effect_order, warnings) = effect_dependencies::plan(
        graph,
        &analysis.modules,
        &evaluation_roots
            .iter()
            .copied()
            .chain(binding_roots.iter().map(|node| match node {
                GraphNode::Module(file) => *file,
                GraphNode::Segment(key) => key.file_id,
                GraphNode::Unlinked { .. } => unreachable!("validated binding root"),
            }))
            .collect::<Vec<_>>(),
        &analysis.effects,
        &mut analysis.edges,
    );
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
        effect_order,
        warnings,
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
    export_names: BTreeMap<u32, BTreeSet<String>>,
    ecco_effects: &'g BTreeMap<SegmentKey, bool>,
    native_externals: bool,
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
            if self.native_externals
                && self.facts(file)?.dependencies.iter().any(|dependency| {
                    !dependency.type_only
                        && matches!(
                            dependency.kind,
                            ast_segmenter::owned::DependencyKind::Import
                                | ast_segmenter::owned::DependencyKind::ReExport
                        )
                        && link::contains(statement.span, dependency.span)
                })
            {
                self.queue.push_back(Demand::Binding(
                    GraphNode::Segment(key),
                    RetentionReason::Dependency,
                ));
                self.edges.insert(RetentionEdge {
                    from: GraphNode::Module(file),
                    to: GraphNode::Segment(key),
                    kind: RetentionEdgeKind::NativeValidation,
                    source_edge: None,
                });
            }
            let summary = effect_analysis::summarize(
                graph,
                file,
                self.facts(file)?,
                statement.span,
                self.values.contains(&key),
                self.policy,
                self.ecco_effects.get(&key).copied(),
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
                    let mut routes = Vec::new();
                    if self.facts(file)?.commonjs {
                        for index in 0..self.graph.file_segments(file).unwrap().len() {
                            routes.push(GraphNode::Segment(SegmentKey::new(file, index as u32)));
                        }
                    } else {
                        if self.graph.has_external_star_exports(file) {
                            for export in self
                                .facts(file)?
                                .exports
                                .iter()
                                .filter(|export| !export.type_only && export.name.is_none())
                            {
                                let index =
                                    self.graph.file_segments(file).unwrap().partition_point(
                                        |segment| segment.span.end <= export.span.start,
                                    );
                                routes
                                    .push(GraphNode::Segment(SegmentKey::new(file, index as u32)));
                            }
                        }
                        for name in &self.export_names[&file] {
                            match self.graph.resolve_export(
                                file,
                                name,
                                source_graph::Namespace::Value,
                            ) {
                                Ok(linked) => routes.extend(linked.route),
                                Err(error)
                                    if !self
                                        .facts(file)?
                                        .exports
                                        .iter()
                                        .any(|e| e.name.as_ref() == Some(name)) =>
                                {
                                    ensure!(
                                        error.to_string().starts_with("ambiguous ")
                                            || error.to_string().starts_with("missing "),
                                        "{error:#}"
                                    );
                                }
                                Err(error) => return Err(error),
                            }
                        }
                    }
                    routes.sort_by_key(|node| node_key(*node));
                    routes.dedup();
                    for target in routes {
                        self.edges.insert(RetentionEdge {
                            from: node,
                            to: target,
                            kind: RetentionEdgeKind::Namespace,
                            source_edge: None,
                        });
                        self.queue.push_back(Demand::Binding(
                            target,
                            RetentionReason::NamespaceConservative,
                        ));
                    }
                }
                return Ok(());
            }
            GraphNode::Unlinked { .. } => {
                self.target(node, true)?;
                return Ok(());
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
            let summary = effect_analysis::summarize(
                self.graph,
                key.file_id,
                self.facts(key.file_id)?,
                span,
                true,
                self.policy,
                self.ecco_effects.get(&key).copied(),
            );
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
