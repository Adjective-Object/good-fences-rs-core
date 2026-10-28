//! Interop demands come from the checked reverse occurrence index, not source guesses.
use std::collections::{BTreeSet, HashMap, HashSet};

use ast_segmenter::owned::{BindingId, ImportedName};
use ast_segmenter::views::ExportValue;
use source_graph::{DependencyTarget, ExportIdentity, GraphNode, Namespace, SourceGraph};

use crate::{RetentionEdgeKind, RetentionPlan};

#[cfg(test)]
thread_local! {
    static REQUEST_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_request_work() {
    REQUEST_WORK.with(|work| work.set(work.get() + 1));
}

/// Scoped to these exact immutable graph/retention/placement inputs, never root demand.
pub(crate) struct Requests<'a> {
    graph: &'a SourceGraph,
    plan: Option<&'a crate::chunks::ChunkPlan>,
    modules: HashSet<u32>,
    segments: HashSet<source_graph::SegmentKey>,
    binding_demands: HashSet<GraphNode>,
    demand_from: HashMap<usize, GraphNode>,
    native_externals: bool,
}

impl<'a> Requests<'a> {
    pub(crate) fn new(
        graph: &'a SourceGraph,
        retention: &'a RetentionPlan,
        plan: Option<&'a crate::chunks::ChunkPlan>,
    ) -> Self {
        let modules = retention
            .modules
            .iter()
            .copied()
            .inspect(|_| {
                #[cfg(test)]
                count_request_work();
            })
            .collect();
        let segments = retention
            .segments
            .iter()
            .map(|s| {
                #[cfg(test)]
                count_request_work();
                s.key
            })
            .collect();
        let mut demand_from = HashMap::new();
        for edge in &retention.edges {
            #[cfg(test)]
            count_request_work();
            if let Some(index) = edge.source_edge {
                // Preserve the first retained origin, as the occurrence scan did.
                demand_from.entry(index).or_insert(edge.from);
            }
        }
        let native_externals = graph.module_ids().any(|file| {
            #[cfg(test)]
            count_request_work();
            crate::link::has_externals(graph, file)
        });
        Self {
            graph,
            plan,
            modules,
            segments,
            binding_demands: binding_demands(retention),
            demand_from,
            native_externals,
        }
    }

    fn retained(&self, node: GraphNode) -> bool {
        #[cfg(test)]
        count_request_work();
        match node {
            GraphNode::Module(file) => self.modules.contains(&file),
            GraphNode::Segment(key) => self.segments.contains(&key),
            GraphNode::Unlinked { .. } => false,
        }
    }

    pub(crate) fn requests(&self, file: u32) -> Vec<(GraphNode, ImportedName)> {
        requests_with_plan(self, file)
    }

    pub(crate) fn names(&self, file: u32) -> BTreeSet<String> {
        let mut names = BTreeSet::from(["default".to_owned()]);
        names.extend(
            self.requests(file)
                .into_iter()
                .filter_map(|(_, imported)| match imported {
                    ImportedName::Name(name) => Some(name),
                    ImportedName::Namespace => None,
                }),
        );
        names
    }
}

fn requests_with_plan(requests: &Requests<'_>, file: u32) -> Vec<(GraphNode, ImportedName)> {
    let graph = requests.graph;
    let plan = requests.plan;
    graph
        .incoming_indexed(GraphNode::Module(file))
        .filter(|(_, edge)| {
            !edge.type_only
                && matches!(
                    edge.kind,
                    source_graph::EdgeKind::Import | source_graph::EdgeKind::ReExport
                )
        })
        .filter_map(|(index, edge)| {
            #[cfg(test)]
            count_request_work();
            let from = match edge.from {
                GraphNode::Module(file) => file,
                GraphNode::Segment(key) => key.file_id,
                GraphNode::Unlinked { .. } => return None,
            };
            let demand_from = if requests.retained(edge.from) {
                edge.from
            } else {
                requests.demand_from.get(&index).copied().or_else(|| {
                    // Namespace routes can bypass an import connector. Only
                    // the planned facade demand certifies its binding use.
                    plan.filter(|_| edge.binding.is_some()).map(|_| edge.from)
                })?
            };
            if !requests.native_externals
                && edge.binding.is_some_and(|binding| {
                    !binding_used(
                        graph,
                        from,
                        binding,
                        None,
                        &requests.binding_demands,
                        |node| requests.retained(node),
                    )
                })
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

pub(crate) fn binding_demands(retention: &RetentionPlan) -> HashSet<GraphNode> {
    retention
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.kind,
                RetentionEdgeKind::Binding
                    | RetentionEdgeKind::Namespace
                    | RetentionEdgeKind::Require
            )
        })
        .map(|edge| edge.to)
        .collect()
}

pub(crate) fn binding_used(
    graph: &SourceGraph,
    file: u32,
    binding: BindingId,
    lazy_modules: Option<&crate::lazy_modules::Certification>,
    binding_demands: &HashSet<GraphNode>,
    retained: impl Fn(GraphNode) -> bool,
) -> bool {
    let Some(facts) = graph.file_facts(file) else {
        return false;
    };
    let segments = graph.file_segments(file).unwrap();
    let segment = |span: oxc_span::Span| {
        let index = segments.partition_point(|s| s.span.end <= span.start);
        source_graph::SegmentKey::new(file, index as u32)
    };
    facts
        .references()
        .iter()
        .enumerate()
        .any(|(index, reference)| {
            if reference.type_only() || reference.binding() != Some(binding) {
                return false;
            }
            let key = segment(reference.span());
            retained(GraphNode::Segment(key))
                && !facts
                    .emission()
                    .segments()
                    .get(key.segment_idx as usize)
                    .is_some_and(|recipe| recipe.connector_only())
                && !lazy_modules.is_some_and(|lazy| lazy.consumed_constructor(key, index as u32))
        })
        || facts.exports().iter().any(|export| {
            // A canonical declaration can be live through a different route.
            // Demand this complete route, not siblings sharing its connector.
            // Explicit demands survive missing nodes for linker diagnostics.
            !export.type_only()
                && graph.runtime_export(file, export.name())
                && export.value() == ExportValue::Binding(binding)
                && retained(GraphNode::Segment(segment(export.span())))
                && export.name().is_some_and(|name| {
                    graph
                        .resolve_export(file, name, Namespace::Value)
                        .is_ok_and(|linked| {
                            linked
                                .route
                                .iter()
                                .all(|node| retained(*node) || binding_demands.contains(node))
                        })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::{reference_reachability, NodeId, RootId, RootSeed};

    #[test]
    fn shared_requests_preserve_root_specific_alias_default_namespace_and_inactive_demands() {
        let graph = SourceGraph::from_owned([
            crate::tests::input(
                "first.mjs",
                "import {a as alias} from 'shared.cjs'; consume(alias);",
            ),
            crate::tests::input(
                "second.mjs",
                "import {b as alias} from 'shared.cjs'; consume(alias);",
            ),
            crate::tests::input(
                "namespace.mjs",
                "import * as ns from 'shared.cjs'; consume(ns['a']);",
            ),
            crate::tests::input(
                "default.mjs",
                "import value from 'shared.cjs'; consume(value);",
            ),
            crate::tests::input(
                "inactive.mjs",
                "import {unused} from 'shared.cjs'; function dead() { consume(unused); }",
            ),
            crate::tests::input(
                "shared.cjs",
                "exports.a = 1; exports.b = 2; exports.unused = 3;",
            ),
        ]);
        let roots: Vec<_> = (0..5)
            .map(|i| RootSeed {
                id: RootId::Entry(format!("entry{i}")),
                evaluation_roots: vec![i],
                binding_roots: vec![],
            })
            .collect();
        let plan =
            crate::chunks::partition(&graph, &roots, &crate::policy::EffectPolicy::default())
                .unwrap();
        let proxy = |name| NodeId::CommonJsProxy {
            module: "shared.cjs".into(),
            name,
        };
        assert_eq!(
            plan.node_roots[&proxy(Some("a".into()))],
            BTreeSet::from([roots[0].id.clone()])
        );
        assert_eq!(
            plan.node_roots[&proxy(Some("b".into()))],
            BTreeSet::from([roots[1].id.clone()])
        );
        assert_eq!(
            plan.node_roots[&NodeId::CommonJsNamespace("shared.cjs".into())],
            BTreeSet::from([roots[2].id.clone()])
        );
        assert_eq!(
            plan.node_roots[&proxy(None)],
            roots
                .iter()
                .map(|root| root.id.clone())
                .collect::<BTreeSet<_>>()
        );
        assert!(!plan.node_roots.contains_key(&proxy(Some("unused".into()))));
        let requests = Requests::new(&graph, &plan.retention, Some(&plan)).requests(5);
        assert!(requests
            .iter()
            .any(|(_, name)| *name == ImportedName::Namespace));
        assert!(requests
            .iter()
            .any(|(_, name)| *name == ImportedName::Name("default".into())));
        assert!(!requests
            .iter()
            .any(|(_, name)| *name == ImportedName::Name("unused".into())));

        let narrowed = crate::chunks::partition(
            &graph,
            &roots[1..2],
            &crate::policy::EffectPolicy::default(),
        )
        .unwrap();
        let narrowed_requests =
            Requests::new(&graph, &narrowed.retention, Some(&narrowed)).requests(5);
        assert_eq!(
            narrowed_requests
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>(),
            vec![
                ImportedName::Name("default".into()),
                ImportedName::Name("b".into())
            ]
        );
    }

    #[test]
    fn occurrence_index_keeps_first_retained_origin_and_original_requested_alias() {
        let graph = SourceGraph::from_owned([
            crate::tests::input("entry.mjs", "import {a as first, a as second} from 'shared.cjs'; consume(first); consume(second);"),
            crate::tests::input("shared.cjs", "exports.a = 1;"),
        ]);
        let mut retention = crate::retain(&graph, &[0], &[]).unwrap();
        let (index, source) = graph
            .incoming_indexed(GraphNode::Module(1))
            .find(|(_, edge)| edge.binding.is_some())
            .unwrap();
        retention
            .segments
            .retain(|segment| GraphNode::Segment(segment.key) != source.from);
        let mut original = retention
            .edges
            .iter()
            .find(|edge| edge.source_edge == Some(index))
            .unwrap()
            .clone();
        original.from = GraphNode::Segment(source_graph::SegmentKey::new(0, 1));
        retention.edges.insert(0, original.clone());
        let mut later = original.clone();
        later.from = GraphNode::Module(1);
        retention.edges.push(later);
        let requests: Vec<_> = Requests::new(&graph, &retention, None)
            .requests(1)
            .into_iter()
            .filter(|(_, name)| *name == ImportedName::Name("a".into()))
            .collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], (original.from, ImportedName::Name("a".into())));
        assert_ne!(
            requests[0].0, requests[1].0,
            "separate source occurrences retain separate consumer origins"
        );
    }

    #[test]
    fn shared_cjs_request_work_is_linear_before_per_root_demand_filtering() {
        const IMPORTERS: u32 = 128;
        const ROOTS: u32 = 16;
        let graph = SourceGraph::from_owned(
            (0..IMPORTERS)
                .map(|i| {
                    crate::tests::input(
                        &format!("entry{i}.mjs"),
                        "import {value as alias} from 'shared.cjs'; consume(alias);",
                    )
                })
                .chain([crate::tests::input("shared.cjs", "exports.value = 7;")]),
        );
        let retention = crate::retain(&graph, &(0..IMPORTERS).collect::<Vec<_>>(), &[]).unwrap();
        let roots: Vec<_> = (0..ROOTS)
            .map(|i| RootSeed {
                id: RootId::Entry(format!("entry{i}")),
                evaluation_roots: vec![i],
                binding_roots: vec![],
            })
            .collect();
        REQUEST_WORK.with(|work| work.set(0));
        let reached = reference_reachability(
            &graph,
            &retention,
            &crate::policy::EffectPolicy::default(),
            &roots,
        )
        .unwrap();
        let work = REQUEST_WORK.with(std::cell::Cell::get);
        for root in &roots {
            assert!(reached[&root.id].contains(&NodeId::CommonJsProxy {
                module: "shared.cjs".into(),
                name: Some("value".into()),
            }));
        }
        eprintln!(
            "CJS request work: {work}; importers={IMPORTERS}, roots={ROOTS}, retained_edges={}, retained_segments={}",
            retention.edges.len(),
            retention.segments.len()
        );
        assert!(work < 32 * IMPORTERS as usize, "request work: {work}");
    }
}
