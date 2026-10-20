use std::{collections::BTreeMap, path::PathBuf};

use ahashmap::{AHashMap, AHashSet};
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
    Unlinked {
        file_id: u32,
        dependency: DependencyId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyTarget {
    File(PathBuf),
    Context(BTreeMap<String, PathBuf>),
    External(String),
    Missing(String),
}

/// Source-local data and environment-dependent resolutions remain separate.
#[derive(Debug)]
pub struct OwnedSourceFileInput {
    pub path: PathBuf,
    pub segments: Vec<Segment>,
    pub facts: ModuleFacts,
    pub resolutions: AHashMap<DependencyId, DependencyTarget>,
    pub provenance: Option<ast_segmenter::SourceProvenance>,
    prepared_resource: Option<PreparedResource>,
}

/// Generated resource identity bound to its original path and extraction mode.
#[derive(Debug)]
pub struct PreparedResource {
    path: PathBuf,
    provenance: ast_segmenter::SourceProvenance,
}

impl PreparedResource {
    pub fn config(
        &self,
        path: &std::path::Path,
        provenance: &ast_segmenter::SourceProvenance,
    ) -> eyre::Result<ast_segmenter::ParserConfig> {
        eyre::ensure!(
            self.path == path && self.provenance == *provenance,
            "prepared resource identity/parser configuration mismatch"
        );
        Ok(self.provenance.config())
    }
}

impl OwnedSourceFileInput {
    pub fn from_parsed(
        path: PathBuf,
        parsed: ast_segmenter::ParsedModule,
        resolutions: AHashMap<DependencyId, DependencyTarget>,
    ) -> Self {
        let provenance = Some(parsed.provenance().clone());
        Self {
            path,
            segments: parsed.segments,
            facts: parsed.facts,
            resolutions,
            provenance,
            prepared_resource: None,
        }
    }

    /// Preserve the original resource identity independently of its JS parser path.
    pub fn from_prepared_resource(
        path: PathBuf,
        parsed: ast_segmenter::ParsedModule,
        resolutions: AHashMap<DependencyId, DependencyTarget>,
    ) -> Self {
        let prepared_resource = PreparedResource {
            path: path.clone(),
            provenance: parsed.provenance().clone(),
        };
        let mut input = Self::from_parsed(path, parsed, resolutions);
        input.prepared_resource = Some(prepared_resource);
        input
    }

    /// Build analysis inputs from owned facts/ranges without legacy tracker maps.
    /// Without extraction provenance this input is analysis-only, not emittable.
    pub fn from_ranges(
        path: PathBuf,
        ranges: impl IntoIterator<Item = Span>,
        facts: ModuleFacts,
        resolutions: AHashMap<DependencyId, DependencyTarget>,
    ) -> Self {
        Self {
            path,
            facts,
            resolutions,
            provenance: None,
            prepared_resource: None,
            segments: ranges
                .into_iter()
                .map(|span| Segment {
                    span,
                    module_deps: Default::default(),
                    variables: Default::default(),
                })
                .collect(),
        }
    }
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
            file.symbol_to_segment.clear();
            for export in &input.facts.exports {
                if let Some(name) = &export.name {
                    if let Some(index) = file.segments.iter().position(|s| {
                        s.span.start <= export.span.start && export.span.end <= s.span.end
                    }) {
                        file.symbol_to_segment.insert(
                            ast_segmenter::ExportedSymbol::from(name.as_str()),
                            index as u32,
                        );
                    }
                }
            }
            file.owned = Some(input);
        }
        graph.rebuild_edges();
        graph
    }

    pub fn edges(&self) -> &[OccurrenceEdge] {
        &self.edges
    }

    /// Checked consumers must distinguish owned inputs from lossy tracker adapters.
    pub fn file_facts(&self, file_id: u32) -> Option<&ModuleFacts> {
        self.file_path(file_id)?;
        self.files
            .get(file_id as usize)?
            .owned
            .as_ref()
            .map(|i| &i.facts)
    }

    pub fn file_provenance(&self, file_id: u32) -> Option<&ast_segmenter::SourceProvenance> {
        self.file_path(file_id)?;
        self.files
            .get(file_id as usize)?
            .owned
            .as_ref()?
            .provenance
            .as_ref()
    }

    pub fn file_prepared_resource(&self, file_id: u32) -> Option<&PreparedResource> {
        self.file_path(file_id)?;
        self.files
            .get(file_id as usize)?
            .owned
            .as_ref()?
            .prepared_resource
            .as_ref()
    }

    pub fn outgoing(&self, node: GraphNode) -> impl Iterator<Item = &OccurrenceEdge> {
        self.outgoing
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }

    /// Indices are valid only at the current graph revision.
    pub fn outgoing_indexed(
        &self,
        node: GraphNode,
    ) -> impl Iterator<Item = (usize, &OccurrenceEdge)> {
        self.outgoing
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&i| (i, &self.edges[i]))
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
            let Some(input) = &file.owned else {
                edges.extend(self.legacy_edges(file_id));
                continue;
            };
            for reference in &input.facts.references {
                let Some(binding) = reference.binding else {
                    continue;
                };
                let Some(declaration) = input.facts.bindings.iter().find(|b| b.id == binding)
                else {
                    continue;
                };
                let from = self.segment_at(file_id, reference.span);
                let mut targets = AHashSet::default();
                for &span in &declaration.declarations {
                    let to = self.segment_at(file_id, span);
                    if targets.insert(to) {
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
                }
            }
            for dependency in &input.facts.dependencies {
                let context = match input.resolutions.get(&dependency.id) {
                    Some(DependencyTarget::Context(candidates)) => Some(candidates),
                    _ => None,
                };
                if dependency.pattern.is_some() != context.is_some()
                    || context.is_some_and(|candidates| {
                        !matches!(
                            dependency.kind,
                            DependencyKind::DynamicImport | DependencyKind::Require
                        ) || candidates
                            .keys()
                            .any(|request| !dependency.matches_specifier(request))
                    })
                {
                    self.diagnostics.push(crate::LinkDiagnostic {
                        file_id,
                        site: dependency.span,
                        message: "dependency context does not match its source pattern".into(),
                    });
                    continue;
                }
                let unresolved = GraphNode::Unlinked {
                    file_id,
                    dependency: dependency.id,
                };
                let target = match input.resolutions.get(&dependency.id) {
                    Some(DependencyTarget::File(path)) => self
                        .file_id(path)
                        .map(GraphNode::Module)
                        .unwrap_or(unresolved),
                    _ => unresolved,
                };
                let failure = match input.resolutions.get(&dependency.id) {
                    Some(DependencyTarget::External(_)) => None,
                    Some(DependencyTarget::Missing(message)) => Some(message.clone()),
                    Some(DependencyTarget::File(path)) if self.file_id(path).is_none() => {
                        Some(format!("target not in graph: {}", path.display()))
                    }
                    Some(DependencyTarget::Context(candidates)) => candidates
                        .values()
                        .find(|path| self.file_id(path).is_none())
                        .map(|path| format!("context target not in graph: {}", path.display())),
                    None => Some(format!("unresolved dependency {:?}", dependency.specifier)),
                    _ => None,
                };
                if let Some(message) = failure {
                    self.diagnostics.push(crate::LinkDiagnostic {
                        file_id,
                        site: dependency.span,
                        message,
                    });
                }
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
                    to: target,
                    kind,
                    purpose: if kind == EdgeKind::ReExport {
                        EdgePurpose::Evaluation
                    } else if bindings.is_empty() {
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
                if let Some(DependencyTarget::Context(candidates)) =
                    input.resolutions.get(&dependency.id)
                {
                    let mut seen = AHashSet::default();
                    for path in candidates.values() {
                        if seen.insert(path) {
                            edges.push(OccurrenceEdge {
                                to: self
                                    .file_id(path)
                                    .map(GraphNode::Module)
                                    .unwrap_or(unresolved),
                                ..base.clone()
                            });
                        }
                    }
                    continue;
                }
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
                let from_file = match edge.from {
                    GraphNode::Segment(k) => k.file_id,
                    GraphNode::Module(id) | GraphNode::Unlinked { file_id: id, .. } => id,
                };
                let namespace = if edge.type_only || self.files[from_file as usize].owned.is_none()
                {
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
                            GraphNode::Unlinked { file_id, .. } => file_id,
                        };
                        self.diagnostics.push(crate::LinkDiagnostic {
                            file_id,
                            site: edge.site,
                            message: error.to_string(),
                        });
                        let mut edge = edge;
                        edge.to = GraphNode::Unlinked {
                            file_id,
                            dependency: edge.dependency.unwrap_or(DependencyId(edge.site.start)),
                        };
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
        OwnedSourceFileInput::from_parsed(path.into(), parsed, resolutions)
    }

    #[test]
    fn local_and_export_demands_include_every_declaration_without_crossing_shadows() {
        let graph = SourceGraph::from_owned([
            input(
                "dep.mjs",
                "var value = 1; var value = 2; var value; function get() { return value; } function shadow() { let value = 3; return value; } var unrelated = 99; export {value, get};",
            ),
            input("main.mjs", "import {value} from 'dep.mjs'; console.log(value);"),
        ]);
        graph.validate_links().unwrap();
        let targets = |segment| {
            graph
                .outgoing(GraphNode::Segment(SegmentKey::new(0, segment)))
                .filter(|edge| edge.kind == EdgeKind::LocalReference)
                .map(|edge| edge.to)
                .collect::<Vec<_>>()
        };
        let declarations: Vec<_> = (0..3)
            .map(|index| GraphNode::Segment(SegmentKey::new(0, index)))
            .collect();
        assert_eq!(targets(3), declarations);
        assert_eq!(targets(4), vec![GraphNode::Segment(SegmentKey::new(0, 4))]);
        let export = graph
            .resolve_export(0, "value", crate::Namespace::Value)
            .unwrap();
        for declaration in declarations {
            assert!(export.route.contains(&declaration));
            assert!(graph
                .outgoing(GraphNode::Segment(SegmentKey::new(1, 0)))
                .any(|edge| edge.purpose == EdgePurpose::Binding && edge.to == declaration));
        }
        assert!(!export
            .route
            .contains(&GraphNode::Segment(SegmentKey::new(0, 5))));
    }

    #[test]
    fn merged_function_var_declarations_and_same_segment_redeclarations() {
        for (format, source, consumer, targets) in [
            (
                ModuleFormat::Esm,
                "export function get() { var value; function value() { return 1; } var value = 2; var value; return value; }",
                0,
                vec![0],
            ),
            (
                ModuleFormat::Esm,
                "function get() { var value = 1; var value = 2; var value; return value; } get();",
                0,
                vec![0],
            ),
        ] {
            let parsed = parse_module(
                &logger::StdioLogger::new(),
                source,
                Path::new("input.js"),
                format,
            )
            .unwrap();
            let graph = SourceGraph::from_owned([OwnedSourceFileInput::from_ranges(
                "input.js".into(),
                parsed.segments.iter().map(|s| s.span),
                parsed.facts,
                Default::default(),
            )]);
            let edges: Vec<_> = graph
                .outgoing(GraphNode::Segment(SegmentKey::new(0, consumer)))
                .filter(|e| e.kind == EdgeKind::LocalReference)
                .map(|e| e.to)
                .collect();
            assert_eq!(
                edges,
                targets.into_iter().map(|i| GraphNode::Segment(SegmentKey::new(0, i))).collect::<Vec<_>>(),
                "{source}",
            );
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
            if edge.purpose == EdgePurpose::Binding {
                assert_eq!(
                    edge.to,
                    GraphNode::Unlinked {
                        file_id: 0,
                        dependency: edge.dependency.unwrap(),
                    }
                );
            } else {
                assert_eq!(edge.to, GraphNode::Module(1));
            }
            assert!(!source[edge.site.start as usize..edge.site.end as usize].is_empty());
        }
    }

    #[test]
    fn contexts_expand_real_occurrences_and_reject_mismatched_resolutions() {
        let mut main = input(
            "main.mjs",
            "export const load = name => import(`./parts/${name}.mjs`);",
        );
        let dependency = main.facts.dependencies[0].id;
        let candidates = BTreeMap::from([
            ("./parts/a.mjs".into(), "a.mjs".into()),
            ("./parts/b.mjs".into(), "b.mjs".into()),
        ]);
        main.resolutions
            .insert(dependency, DependencyTarget::Context(candidates));
        let graph = SourceGraph::from_owned([main, input("a.mjs", ""), input("b.mjs", "")]);
        graph.validate_links().unwrap();
        let edges: Vec<_> = graph
            .edges()
            .iter()
            .filter(|edge| edge.dependency == Some(dependency))
            .collect();
        assert_eq!(edges.len(), 2);
        for edge in edges {
            assert_eq!(edge.activation, Activation::Asynchronous);
            assert_eq!(
                graph
                    .incoming(edge.to)
                    .filter(|incoming| *incoming == edge)
                    .count(),
                1
            );
        }

        let main = input(
            "main.mjs",
            "export const load = name => import(`./parts/${name}.mjs`);",
        );
        assert!(SourceGraph::from_owned([main]).validate_links().is_err());
        let mut main = input("main.mjs", "import './a.mjs';");
        main.resolutions.insert(
            main.facts.dependencies[0].id,
            DependencyTarget::Context(BTreeMap::new()),
        );
        assert!(SourceGraph::from_owned([main]).validate_links().is_err());
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
            OwnedSourceFileInput::from_parsed("entry.cjs".into(), parsed, resolutions),
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
