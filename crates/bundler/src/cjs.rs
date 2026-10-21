//! Interop demands come from the checked reverse occurrence index, not source guesses.
use std::collections::BTreeSet;

use ast_segmenter::owned::{BindingId, ImportedName};
use ast_segmenter::views::ExportValue;
use source_graph::{DependencyTarget, ExportIdentity, GraphNode, Namespace, SourceGraph};

use crate::RetentionPlan;

pub(crate) fn requests(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
) -> Vec<(GraphNode, ImportedName)> {
    requests_with_plan(graph, retention, file, None)
}

pub(crate) fn planned_requests(
    graph: &SourceGraph,
    plan: &crate::chunks::ChunkPlan,
    file: u32,
) -> Vec<(GraphNode, ImportedName)> {
    requests_with_plan(graph, &plan.retention, file, Some(plan))
}

fn requests_with_plan(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    plan: Option<&crate::chunks::ChunkPlan>,
) -> Vec<(GraphNode, ImportedName)> {
    let native_externals = graph
        .module_ids()
        .any(|file| crate::link::has_externals(graph, file));
    graph
        .incoming(GraphNode::Module(file))
        .filter(|edge| {
            !edge.type_only
                && matches!(
                    edge.kind,
                    source_graph::EdgeKind::Import | source_graph::EdgeKind::ReExport
                )
        })
        .filter_map(|edge| {
            let from = match edge.from {
                GraphNode::Module(file) => file,
                GraphNode::Segment(key) => key.file_id,
                GraphNode::Unlinked { .. } => return None,
            };
            let demand_from = if retained(retention, edge.from) {
                edge.from
            } else {
                retention
                    .edges
                    .iter()
                    .find(|retained| {
                        retained
                            .source_edge
                            .is_some_and(|index| std::ptr::eq(&graph.edges()[index], edge))
                    })
                    .map(|retained| retained.from)
                    .or_else(|| {
                        // Namespace routes can bypass an import connector. Only
                        // the planned facade demand certifies its binding use.
                        plan.filter(|_| edge.binding.is_some()).map(|_| edge.from)
                    })?
            };
            if !native_externals
                && edge
                    .binding
                    .is_some_and(|binding| !binding_used(graph, retention, from, binding))
            {
                return None;
            }
            let DependencyTarget::File(path) = graph.dependency_target(from, edge.dependency?)?
            else {
                return None;
            };
            let target = graph.file_id(path)?;
            let imported = match edge.imported.as_ref() {
                Some(ImportedName::Name(name)) => graph
                    .resolve_export_alternatives(target, name, Namespace::Value)
                    .ok()?
                    .into_iter()
                    .filter_map(|candidate| match candidate.identity {
                        ExportIdentity::CommonJs(target, name) if target == file => {
                            Some(ImportedName::Name(name))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                Some(ImportedName::Namespace) if target == file => vec![ImportedName::Namespace],
                None if target == file => vec![ImportedName::Name("default".into())],
                _ => return None,
            };
            Some(
                imported
                    .into_iter()
                    .filter(|imported| {
                        plan.is_none_or(|plan| {
                            let module = graph.file_path(file).unwrap().to_owned();
                            let node = match imported {
                                ImportedName::Namespace => {
                                    crate::chunks::NodeId::CommonJsNamespace(module)
                                }
                                ImportedName::Name(name) => crate::chunks::NodeId::CommonJsProxy {
                                    module,
                                    name: (name != "default").then(|| name.clone()),
                                },
                            };
                            plan.chunk_for(&node).is_some()
                        })
                    })
                    .map(|imported| (demand_from, imported))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

pub(crate) fn binding_used(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    binding: BindingId,
) -> bool {
    let Some(facts) = graph.file_facts(file) else {
        return false;
    };
    let segments = graph.file_segments(file).unwrap();
    let live = |span: oxc_span::Span| {
        let index = segments.partition_point(|s| s.span.end <= span.start);
        retained(
            retention,
            GraphNode::Segment(source_graph::SegmentKey::new(file, index as u32)),
        )
    };
    facts
        .references()
        .iter()
        .any(|r| !r.type_only() && r.binding() == Some(binding) && live(r.span()))
        || facts
            .exports()
            .iter()
            .any(|e| !e.type_only() && e.value() == ExportValue::Binding(binding) && live(e.span()))
}

fn retained(retention: &RetentionPlan, node: GraphNode) -> bool {
    match node {
        GraphNode::Module(file) => retention.modules.contains(&file),
        GraphNode::Segment(key) => retention.segments.iter().any(|s| s.key == key),
        GraphNode::Unlinked { .. } => false,
    }
}

pub(crate) fn names(graph: &SourceGraph, retention: &RetentionPlan, file: u32) -> BTreeSet<String> {
    let mut names = BTreeSet::from(["default".to_owned()]);
    names.extend(
        requests(graph, retention, file)
            .into_iter()
            .filter_map(|(_, imported)| match imported {
                ImportedName::Name(name) => Some(name),
                ImportedName::Namespace => None,
            }),
    );
    names
}

pub(crate) fn planned_names(
    graph: &SourceGraph,
    plan: &crate::chunks::ChunkPlan,
    file: u32,
) -> BTreeSet<String> {
    let mut names = BTreeSet::from(["default".to_owned()]);
    names.extend(
        planned_requests(graph, plan, file).into_iter().filter_map(
            |(_, imported)| match imported {
                ImportedName::Name(name) => Some(name),
                ImportedName::Namespace => None,
            },
        ),
    );
    names
}
