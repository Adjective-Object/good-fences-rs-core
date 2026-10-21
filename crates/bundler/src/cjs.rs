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
