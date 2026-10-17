use std::path::PathBuf;

use ahashmap::AHashMap;
use ast_segmenter::owned::{
    BindingId, DependencyId, DependencyKind, ExportValue, ImportedName, ModuleFacts,
};
use ast_segmenter::segment_info::Segment;
use oxc_span::Span;

use crate::{SegmentKey, SourceFileInput, SourceGraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphNode {
    Module(u32),
    Segment(SegmentKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyTarget {
    File(PathBuf),
    External(String),
    Missing(String),
}

/// Source-local data and environment-dependent resolutions remain separate.
pub struct OwnedSourceFileInput {
    pub path: PathBuf,
    pub segments: Vec<Segment>,
    pub facts: ModuleFacts,
    pub resolutions: AHashMap<DependencyId, DependencyTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    LocalReference,
    Import,
    ReExport,
    DynamicImport,
    Require,
    ImportType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePurpose {
    Binding,
    Evaluation,
    Execution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Synchronous,
    Asynchronous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceEdge {
    pub from: GraphNode,
    pub to: GraphNode,
    pub kind: EdgeKind,
    pub purpose: EdgePurpose,
    pub activation: Activation,
    pub type_only: bool,
    pub site: Span,
    pub dependency: Option<DependencyId>,
    pub enclosing_function: Option<Span>,
    pub imported: Option<ImportedName>,
    pub binding: Option<BindingId>,
    pub read: bool,
    pub write: bool,
}

impl SourceGraph {
    pub fn from_owned(inputs: impl IntoIterator<Item = OwnedSourceFileInput>) -> Self {
        let inputs: Vec<_> = inputs.into_iter().collect();
        let mut graph = Self::new(inputs.iter().map(|input| {
            SourceFileInput {
                source_file_path: input.path.clone(),
                segments: input.segments.clone(),
                resolved_reexport_paths: input
                    .facts
                    .dependencies
                    .iter()
                    .filter_map(|d| match input.resolutions.get(&d.id) {
                        Some(DependencyTarget::File(p)) => Some((d.specifier.clone(), p.clone())),
                        _ => None,
                    })
                    .collect(),
                resolved_import_paths: AHashMap::default(),
            }
        }));
        for (file, input) in graph.files.iter_mut().zip(inputs) {
            file.owned = Some(input);
        }
        graph.rebuild_edges();
        graph
    }

    pub fn edges(&self) -> &[OccurrenceEdge] {
        &self.edges
    }

    pub fn outgoing(&self, node: GraphNode) -> impl Iterator<Item = &OccurrenceEdge> {
        self.outgoing
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }

    pub fn incoming(&self, node: GraphNode) -> impl Iterator<Item = &OccurrenceEdge> {
        self.incoming
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }

    pub(crate) fn segment_at(&self, file_id: u32, span: Span) -> GraphNode {
        self.files[file_id as usize]
            .segments
            .iter()
            .position(|s| s.span.start <= span.start && span.end <= s.span.end)
            .map_or(GraphNode::Module(file_id), |i| {
                GraphNode::Segment(SegmentKey::new(file_id, i as u32))
            })
    }

    pub(crate) fn rebuild_edges(&mut self) {
        self.diagnostics.clear();
        let mut edges = Vec::new();
        for (file_id, file) in self.files.iter().enumerate() {
            let file_id = file_id as u32;
            let Some(input) = &file.owned else { continue };
            for reference in &input.facts.references {
                let Some(binding) = reference.binding else {
                    continue;
                };
                let Some(declaration) = input.facts.bindings.iter().find(|b| b.id == binding)
                else {
                    continue;
                };
                let Some(&span) = declaration.declarations.first() else {
                    continue;
                };
                let from = self.segment_at(file_id, reference.span);
                let to = self.segment_at(file_id, span);
                edges.push(OccurrenceEdge {
                    from,
                    to,
                    kind: EdgeKind::LocalReference,
                    purpose: EdgePurpose::Binding,
                    activation: Activation::Synchronous,
                    type_only: reference.type_only,
                    site: reference.span,
                    dependency: None,
                    enclosing_function: None,
                    imported: None,
                    binding: Some(binding),
                    read: reference.read,
                    write: reference.write,
                });
            }
            for dependency in &input.facts.dependencies {
                let Some(DependencyTarget::File(path)) = input.resolutions.get(&dependency.id)
                else {
                    continue;
                };
                let Some(target) = self.file_id(path) else {
                    continue;
                };
                let kind = match dependency.kind {
                    DependencyKind::Import => EdgeKind::Import,
                    DependencyKind::ReExport => EdgeKind::ReExport,
                    DependencyKind::DynamicImport => EdgeKind::DynamicImport,
                    DependencyKind::Require => EdgeKind::Require,
                    DependencyKind::ImportType => EdgeKind::ImportType,
                };
                let bindings: Vec<_> = input
                    .facts
                    .imports
                    .iter()
                    .filter(|i| i.dependency == dependency.id)
                    .collect();
                let base = OccurrenceEdge {
                    from: self.segment_at(file_id, dependency.span),
                    to: GraphNode::Module(target),
                    kind,
                    purpose: if bindings.is_empty() {
                        EdgePurpose::Execution
                    } else {
                        EdgePurpose::Evaluation
                    },
                    activation: if kind == EdgeKind::DynamicImport {
                        Activation::Asynchronous
                    } else {
                        Activation::Synchronous
                    },
                    type_only: dependency.type_only,
                    site: dependency.span,
                    dependency: Some(dependency.id),
                    enclosing_function: dependency.enclosing_function,
                    imported: None,
                    binding: None,
                    read: false,
                    write: false,
                };
                edges.push(base.clone());
                for binding in bindings {
                    edges.push(OccurrenceEdge {
                        purpose: EdgePurpose::Binding,
                        imported: Some(binding.imported.clone()),
                        binding: Some(binding.binding),
                        type_only: binding.type_only,
                        site: binding.span,
                        ..base.clone()
                    });
                }
                for export in &input.facts.exports {
                    if let ExportValue::ReExport {
                        dependency: id,
                        imported,
                    } = &export.value
                    {
                        if *id == dependency.id {
                            edges.push(OccurrenceEdge {
                                purpose: EdgePurpose::Binding,
                                imported: Some(imported.clone()),
                                type_only: export.type_only,
                                site: export.span,
                                ..base.clone()
                            });
                        }
                    }
                }
            }
        }
        let mut linked = Vec::new();
        for edge in edges {
            if let (GraphNode::Module(target), Some(ImportedName::Name(name))) =
                (edge.to, &edge.imported)
            {
                let namespace = if edge.type_only {
                    crate::Namespace::Type
                } else {
                    crate::Namespace::Value
                };
                match self.resolve_export(target, name, namespace) {
                    Ok(export) => {
                        for to in export.route {
                            let mut edge = edge.clone();
                            edge.to = to;
                            linked.push(edge);
                        }
                    }
                    Err(error) => {
                        let file_id = match edge.from {
                            GraphNode::Module(id) => id,
                            GraphNode::Segment(key) => key.file_id,
                        };
                        self.diagnostics.push(crate::LinkDiagnostic {
                            file_id,
                            site: edge.site,
                            message: error.to_string(),
                        });
                        linked.push(edge);
                    }
                }
            } else {
                linked.push(edge);
            }
        }
        self.edges = linked;
        self.outgoing.clear();
        self.incoming.clear();
        for (i, edge) in self.edges.iter().enumerate() {
            self.outgoing.entry(edge.from).or_default().push(i);
            self.incoming.entry(edge.to).or_default().push(i);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ast_segmenter::{parse_module, ModuleFormat};
    use std::path::Path;

    pub(crate) fn input(path: &str, source: &str) -> OwnedSourceFileInput {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new(path),
            ModuleFormat::Esm,
        )
        .unwrap();
        let resolutions = parsed
            .facts
            .dependencies
            .iter()
            .map(|d| {
                (
                    d.id,
                    DependencyTarget::File(PathBuf::from(d.specifier.clone())),
                )
            })
            .collect();
        OwnedSourceFileInput {
            path: path.into(),
            segments: parsed.segments,
            facts: parsed.facts,
            resolutions,
        }
    }

    #[test]
    fn occurrences_reverse_parity_and_empty_identity() {
        let source = "import { x as a } from 'empty.ts'; import { x as b } from 'empty.ts'; import('empty.ts'); import('empty.ts');";
        let graph = SourceGraph::from_owned([input("entry.ts", source), input("empty.ts", "")]);
        assert_eq!(graph.file_count(), 2);
        assert!(graph.file_segments(1).unwrap().is_empty());
        let sites: Vec<_> = graph
            .edges()
            .iter()
            .filter(|e| e.purpose != EdgePurpose::Binding)
            .collect();
        assert_eq!(sites.len(), 4);
        assert_ne!(sites[0].dependency, sites[1].dependency);
        assert_ne!(sites[2].dependency, sites[3].dependency);
        assert_eq!(sites[2].activation, Activation::Asynchronous);
        for edge in graph.edges() {
            assert_eq!(graph.outgoing(edge.from).filter(|e| *e == edge).count(), 1);
            assert_eq!(graph.incoming(edge.to).filter(|e| *e == edge).count(), 1);
            assert_eq!(edge.to, GraphNode::Module(1));
            assert!(!source[edge.site.start as usize..edge.site.end as usize].is_empty());
        }
    }

    #[test]
    fn deferred_require_is_sync_but_not_eager_and_references_keep_flags() {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            "let x = 1; x += 2; function later() { return require('empty.ts'); }",
            Path::new("entry.cjs"),
            ModuleFormat::CommonJs,
        )
        .unwrap();
        let resolutions = parsed
            .facts
            .dependencies
            .iter()
            .map(|d| (d.id, DependencyTarget::File("empty.ts".into())))
            .collect();
        let graph = SourceGraph::from_owned([
            OwnedSourceFileInput {
                path: "entry.cjs".into(),
                segments: parsed.segments,
                facts: parsed.facts,
                resolutions,
            },
            input("empty.ts", ""),
        ]);
        let require = graph
            .edges()
            .iter()
            .find(|e| e.kind == EdgeKind::Require)
            .unwrap();
        assert_eq!(require.activation, Activation::Synchronous);
        assert!(require.enclosing_function.is_some());
        assert!(graph
            .edges()
            .iter()
            .any(|e| e.kind == EdgeKind::LocalReference && e.write));
    }
}
