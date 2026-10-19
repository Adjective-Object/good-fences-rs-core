//! Interop demands come from the checked reverse occurrence index, not source guesses.
use std::collections::BTreeSet;

use ast_segmenter::owned::{BindingId, ExportValue, ImportedName};
use source_graph::{DependencyTarget, ExportIdentity, GraphNode, Namespace, SourceGraph};

use crate::RetentionPlan;

pub(crate) fn requests(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
) -> Vec<(GraphNode, ImportedName)> {
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
                    })?
                    .from
            };
            let from = match edge.from {
                GraphNode::Module(file) => file,
                GraphNode::Segment(key) => key.file_id,
                GraphNode::Unlinked { .. } => return None,
            };
            if edge
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
                Some(ImportedName::Name(name)) => {
                    let ExportIdentity::CommonJs(target, name) = graph
                        .resolve_export(target, name, Namespace::Value)
                        .ok()?
                        .identity
                    else {
                        return None;
                    };
                    if target != file {
                        return None;
                    }
                    ImportedName::Name(name)
                }
                Some(ImportedName::Namespace) if target == file => ImportedName::Namespace,
                None if target == file => ImportedName::Name("default".into()),
                _ => return None,
            };
            Some((demand_from, imported))
        })
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
        .references
        .iter()
        .any(|r| !r.type_only && r.binding == Some(binding) && live(r.span))
        || facts
            .exports
            .iter()
            .any(|e| !e.type_only && e.value == ExportValue::Binding(binding) && live(e.span))
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
