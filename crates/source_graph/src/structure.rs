use std::{collections::BTreeMap, path::PathBuf};

use ahashmap::{AHashMap, AHashSet};
use ast_segmenter::owned::{BindingId, DependencyId, DependencyKind, ImportedName};
use ast_segmenter::segment_info::Segment;
use ast_segmenter::views::{ExportValue, FactsStorage, ModuleFacts};
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

/// Attempt-derived removal of proven type-only connectors, never source facts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TypeOnlyWiring {
    pub imports: AHashSet<BindingId>,
    pub exports: AHashSet<String>,
    pub dependencies: AHashSet<DependencyId>,
}

/// Source-local data and environment-dependent resolutions remain separate.
#[derive(Debug)]
pub struct OwnedSourceFileInput {
    pub path: PathBuf,
    pub segments: Vec<Segment>,
    pub facts: FactsStorage,
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
        let (_, _, segments, facts, provenance) = parsed.into_parts();
        Self {
            path,
            segments,
            facts,
            resolutions,
            provenance: Some(provenance),
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
        facts: impl Into<FactsStorage>,
        resolutions: AHashMap<DependencyId, DependencyTarget>,
    ) -> Self {
        Self {
            path,
            facts: facts.into(),
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
    /// Source-reference syntax context, not dependency activation timing.
    pub async_deferred: bool,
}

impl SourceGraph {
    /// Admit caller-proven type-only connectors, rejecting executable references
    /// and attempts to remove explicit evaluation-only clauses.
    pub fn set_type_only_wiring(
        &mut self,
        wiring: AHashMap<u32, TypeOnlyWiring>,
    ) -> eyre::Result<()> {
        if self.type_only_wiring == wiring {
            return Ok(());
        }
        for (&file, mask) in &wiring {
            let facts = self
                .file_facts(file)
                .ok_or_else(|| eyre::eyre!("missing type-only wiring source {file}"))?;
            for &binding in &mask.imports {
                eyre::ensure!(
                    facts.imports().iter().any(|i| i.binding() == binding),
                    "type-only mask is not an import binding"
                );
                eyre::ensure!(
                    !facts
                        .references()
                        .iter()
                        .any(|r| r.binding() == Some(binding)
                            && !r.type_only()
                            && !facts.exports().iter().any(|e| e.value()
                                == ExportValue::Binding(binding)
                                && e.name().is_some_and(|n| mask.exports.contains(n))
                                && e.span().start <= r.span().start
                                && r.span().end <= e.span().end)),
                    "type-only import binding has executable references"
                );
            }
            for &dependency in &mask.dependencies {
                let imports: Vec<_> = facts
                    .imports()
                    .iter()
                    .filter(|i| i.dependency() == dependency)
                    .collect();
                let exports: Vec<_> = facts.exports().iter().filter(|e| matches!(e.value(), ExportValue::ReExport { dependency: id, .. } if id == dependency)).collect();
                eyre::ensure!(
                    (!imports.is_empty() || !exports.is_empty())
                        && imports.iter().all(|i| mask.imports.contains(&i.binding()))
                        && exports
                            .iter()
                            .all(|e| e.name().is_some_and(|n| mask.exports.contains(n))),
                    "type-only dependency mask would remove runtime wiring or explicit evaluation"
                );
            }
        }
        self.type_only_wiring = wiring;
        self.revision.resolution += 1;
        self.reexport_cache.lock().unwrap().clear();
        self.rebuild_edges();
        Ok(())
    }

    pub fn runtime_import(&self, file: u32, binding: BindingId) -> bool {
        !self
            .type_only_wiring
            .get(&file)
            .is_some_and(|w| w.imports.contains(&binding))
    }

    pub fn runtime_export(&self, file: u32, name: Option<&str>) -> bool {
        !name.is_some_and(|name| {
            self.type_only_wiring
                .get(&file)
                .is_some_and(|w| w.exports.contains(name))
        })
    }

    pub fn runtime_dependency(&self, file: u32, dependency: DependencyId) -> bool {
        !self
            .type_only_wiring
            .get(&file)
            .is_some_and(|w| w.dependencies.contains(&dependency))
    }

    pub fn from_owned(inputs: impl IntoIterator<Item = OwnedSourceFileInput>) -> Self {
        let inputs: Vec<_> = inputs.into_iter().collect();
        let mut graph = Self::new(inputs.iter().map(|input| {
            SourceFileInput {
                source_file_path: input.path.clone(),
                segments: input.segments.clone(),
                resolved_reexport_paths: input
                    .facts
                    .dependencies()
                    .iter()
                    .filter_map(|d| match input.resolutions.get(&d.id()) {
                        Some(DependencyTarget::File(p)) => {
                            Some((d.specifier().to_owned(), p.clone()))
                        }
                        _ => None,
                    })
                    .collect(),
                resolved_import_paths: AHashMap::default(),
            }
        }));
        for (file, input) in graph.files.iter_mut().zip(inputs) {
            file.symbol_to_segment.clear();
            for export in input.facts.exports() {
                if let Some(name) = export.name() {
                    if let Some(index) = file.segments.iter().position(|s| {
                        s.span.start <= export.span().start && export.span().end <= s.span.end
                    }) {
                        file.symbol_to_segment
                            .insert(ast_segmenter::ExportedSymbol::from(name), index as u32);
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

    /// Checked consumers must distinguish full extraction from lossy tracker adapters.
    pub fn file_facts(&self, file_id: u32) -> Option<ModuleFacts<'_>> {
        self.live_file(file_id)?
            .owned
            .as_ref()
            .map(|i| i.facts.view())
    }

    pub fn file_fact_storage(&self, file_id: u32) -> Option<&FactsStorage> {
        self.live_file(file_id)?.owned.as_ref().map(|i| &i.facts)
    }

    pub fn file_provenance(&self, file_id: u32) -> Option<&ast_segmenter::SourceProvenance> {
        self.live_file(file_id)?.owned.as_ref()?.provenance.as_ref()
    }

    pub fn file_prepared_resource(&self, file_id: u32) -> Option<&PreparedResource> {
        self.live_file(file_id)?
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
        self.incoming_indexed(node).map(|(_, edge)| edge)
    }

    /// Indices are valid only at the current graph revision.
    pub fn incoming_indexed(
        &self,
        node: GraphNode,
    ) -> impl Iterator<Item = (usize, &OccurrenceEdge)> {
        self.incoming
            .get(&node)
            .into_iter()
            .flatten()
            .map(|&i| (i, &self.edges[i]))
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
            if !self.live_files[file_id] {
                continue;
            }
            let file_id = file_id as u32;
            let Some(input) = &file.owned else {
                edges.extend(self.legacy_edges(file_id));
                continue;
            };
            for reference in input.facts.references() {
                let Some(binding) = reference.binding() else {
                    continue;
                };
                if !self.runtime_import(file_id, binding) {
                    continue;
                }
                let Some(declaration) = input.facts.bindings().iter().find(|b| b.id() == binding)
                else {
                    continue;
                };
                let from = self.segment_at(file_id, reference.span());
                let mut targets = AHashSet::default();
                for span in declaration.declarations() {
                    let to = self.segment_at(file_id, span);
                    if targets.insert(to) {
                        edges.push(OccurrenceEdge {
                            from,
                            to,
                            kind: EdgeKind::LocalReference,
                            purpose: EdgePurpose::Binding,
                            activation: Activation::Synchronous,
                            type_only: reference.type_only(),
                            site: reference.span(),
                            dependency: None,
                            enclosing_function: None,
                            imported: None,
                            binding: Some(binding),
                            read: reference.read(),
                            write: reference.write(),
                            async_deferred: reference.async_deferred(),
                        });
                    }
                }
            }
            for dependency in input.facts.dependencies() {
                if !self.runtime_dependency(file_id, dependency.id()) {
                    continue;
                }
                if dependency.kind() == DependencyKind::Url {
                    // Resource URLs belong to a different realm, not this evaluation graph.
                    if !matches!(
                        input.resolutions.get(&dependency.id()),
                        Some(DependencyTarget::File(_))
                    ) {
                        self.diagnostics.push(crate::LinkDiagnostic {
                            file_id,
                            site: dependency.span(),
                            message: "unresolved resource URL".into(),
                            missing_export: None,
                        });
                    }
                    continue;
                }
                let context = match input.resolutions.get(&dependency.id()) {
                    Some(DependencyTarget::Context(candidates)) => Some(candidates),
                    _ => None,
                };
                if dependency.pattern().is_some() != context.is_some()
                    || context.is_some_and(|candidates| {
                        !matches!(
                            dependency.kind(),
                            DependencyKind::DynamicImport | DependencyKind::Require
                        ) || candidates
                            .keys()
                            .any(|request| !dependency.matches_specifier(request))
                    })
                {
                    self.diagnostics.push(crate::LinkDiagnostic {
                        file_id,
                        site: dependency.span(),
                        message: "dependency context does not match its source pattern".into(),
                        missing_export: None,
                    });
                    continue;
                }
                let unresolved = GraphNode::Unlinked {
                    file_id,
                    dependency: dependency.id(),
                };
                let target = match input.resolutions.get(&dependency.id()) {
                    Some(DependencyTarget::File(path)) => self
                        .file_id(path)
                        .map(GraphNode::Module)
                        .unwrap_or(unresolved),
                    _ => unresolved,
                };
                let failure = match input.resolutions.get(&dependency.id()) {
                    Some(DependencyTarget::External(_)) => None,
                    Some(DependencyTarget::Missing(message)) => Some(message.clone()),
                    Some(DependencyTarget::File(path)) if self.file_id(path).is_none() => {
                        Some(format!("target not in graph: {}", path.display()))
                    }
                    Some(DependencyTarget::Context(candidates)) => candidates
                        .values()
                        .find(|path| self.file_id(path).is_none())
                        .map(|path| format!("context target not in graph: {}", path.display())),
                    None => Some(format!(
                        "unresolved dependency {:?}",
                        dependency.specifier()
                    )),
                    _ => None,
                };
                if let Some(message) = failure {
                    self.diagnostics.push(crate::LinkDiagnostic {
                        file_id,
                        site: dependency.span(),
                        message,
                        missing_export: None,
                    });
                }
                let kind = match dependency.kind() {
                    DependencyKind::Import | DependencyKind::ImportMetaEnv => EdgeKind::Import,
                    DependencyKind::ReExport => EdgeKind::ReExport,
                    DependencyKind::DynamicImport => EdgeKind::DynamicImport,
                    DependencyKind::Require => EdgeKind::Require,
                    DependencyKind::ImportType => EdgeKind::ImportType,
                    DependencyKind::Url => unreachable!("resource URLs have no module edges"),
                };
                let bindings: Vec<_> = input
                    .facts
                    .imports()
                    .iter()
                    .filter(|i| i.dependency() == dependency.id())
                    .filter(|i| self.runtime_import(file_id, i.binding()))
                    .collect();
                let base = OccurrenceEdge {
                    from: self.segment_at(file_id, dependency.span()),
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
                    type_only: dependency.type_only(),
                    site: dependency.span(),
                    dependency: Some(dependency.id()),
                    enclosing_function: dependency.enclosing_function(),
                    imported: None,
                    binding: None,
                    read: false,
                    write: false,
                    async_deferred: false,
                };
                if let Some(DependencyTarget::Context(candidates)) =
                    input.resolutions.get(&dependency.id())
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
                        imported: Some(binding.imported().to_owned()),
                        binding: Some(binding.binding()),
                        type_only: binding.type_only(),
                        site: binding.span(),
                        ..base.clone()
                    });
                }
                for export in input.facts.exports() {
                    if !self.runtime_export(file_id, export.name()) {
                        continue;
                    }
                    if let ExportValue::ReExport {
                        dependency: id,
                        imported,
                    } = &export.value()
                    {
                        if *id == dependency.id() {
                            edges.push(OccurrenceEdge {
                                purpose: EdgePurpose::Binding,
                                imported: Some((*imported).to_owned()),
                                type_only: export.type_only(),
                                site: export.span(),
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
                            missing_export: self
                                .resolve_optional_export(target, name, namespace)
                                .ok()
                                .filter(Option::is_none)
                                .map(|_| crate::MissingExport {
                                    target,
                                    name: name.clone(),
                                    namespace,
                                }),
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
            .facts()
            .dependencies()
            .iter()
            .map(|d| {
                (
                    d.id(),
                    DependencyTarget::File(PathBuf::from(d.specifier().to_owned())),
                )
            })
            .collect();
        OwnedSourceFileInput::from_parsed(path.into(), parsed, resolutions)
    }

    #[test]
    fn type_only_wiring_removes_alias_and_evaluation_edges_without_mutating_facts() {
        let mut graph = SourceGraph::from_owned([
            input("types.mjs", "export const keep=42;"),
            input("main.mjs", "import {Type,keep} from 'types.mjs'; export {Type as Alias,keep}; import 'types.mjs';"),
        ]);
        let facts = graph.file_facts(1).unwrap();
        let binding = facts
            .imports()
            .iter()
            .find(|i| i.imported() == ast_segmenter::views::ImportedName::Name("Type"))
            .unwrap()
            .binding();
        let mask = TypeOnlyWiring {
            imports: AHashSet::from_iter([binding]),
            exports: AHashSet::from_iter(["Alias".into()]),
            dependencies: Default::default(),
        };
        let revision = graph.revision();
        graph
            .set_type_only_wiring(AHashMap::from_iter([(1, mask)]))
            .unwrap();
        graph.validate_links().unwrap();
        assert_ne!(graph.revision(), revision);
        assert_eq!(graph.file_facts(1).unwrap().imports().len(), 2);
        assert!(!graph.edges().iter().any(|e| e.binding == Some(binding)));
        assert!(graph
            .resolve_optional_export(1, "Alias", crate::Namespace::Value)
            .unwrap()
            .is_none());
        assert!(graph
            .resolve_export(1, "keep", crate::Namespace::Value)
            .is_ok());
        assert!(graph
            .edges()
            .iter()
            .any(|e| e.purpose == EdgePurpose::Execution));
    }

    #[test]
    fn type_only_wiring_rejects_executable_reads_and_explicit_side_effect_removal() {
        for source in [
            "import {Type} from 'types.mjs'; console.log(Type);",
            "import 'types.mjs';",
        ] {
            let mut graph =
                SourceGraph::from_owned([input("types.mjs", ""), input("main.mjs", source)]);
            let facts = graph.file_facts(1).unwrap();
            let mask = TypeOnlyWiring {
                imports: facts.imports().iter().map(|i| i.binding()).collect(),
                dependencies: facts.dependencies().iter().map(|d| d.id()).collect(),
                exports: Default::default(),
            };
            assert!(graph
                .set_type_only_wiring(AHashMap::from_iter([(1, mask)]))
                .is_err());
        }
    }

    #[test]
    fn async_deferred_reference_occurrences_keep_mixed_contexts_and_both_indexes() {
        let source = "const target = 1; async function caller() { target; (() => target)(); target; const local = 2; local; }";
        let graph = SourceGraph::from_owned([input("main.mjs", source)]);
        graph.validate_links().unwrap();
        let caller = GraphNode::Segment(SegmentKey::new(0, 1));
        let target = GraphNode::Segment(SegmentKey::new(0, 0));
        let references: Vec<_> = graph
            .outgoing_indexed(caller)
            .filter(|(_, edge)| edge.kind == EdgeKind::LocalReference && edge.to == target)
            .collect();
        assert_eq!(
            references
                .iter()
                .map(|(_, edge)| edge.async_deferred)
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        assert!(references
            .windows(2)
            .all(|pair| pair[0].1.site.start < pair[1].1.site.start));
        for (index, edge) in references {
            assert_eq!(graph.edges()[index], *edge);
            assert!(graph
                .incoming(target)
                .any(|reverse| std::ptr::eq(edge, reverse)));
            let reference = graph
                .file_facts(0)
                .unwrap()
                .references()
                .iter()
                .find(|reference| reference.span() == edge.site)
                .unwrap();
            assert_eq!(edge.async_deferred, reference.async_deferred());
        }
        let same_segment: Vec<_> = graph
            .outgoing(caller)
            .filter(|edge| edge.to == caller)
            .collect();
        assert_eq!(same_segment.len(), 1);
        assert!(same_segment[0].async_deferred);
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
            let (_, _, segments, facts, _) = parsed.into_parts();
            let graph = SourceGraph::from_owned([OwnedSourceFileInput::from_ranges(
                "input.js".into(),
                segments.iter().map(|s| s.span),
                facts,
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
        for (index, edge) in graph.edges().iter().enumerate() {
            assert_eq!(graph.outgoing(edge.from).filter(|e| *e == edge).count(), 1);
            assert_eq!(graph.incoming(edge.to).filter(|e| *e == edge).count(), 1);
            assert_eq!(
                graph
                    .incoming_indexed(edge.to)
                    .filter(|(i, e)| *i == index && std::ptr::eq(*e, edge))
                    .count(),
                1
            );
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

    fn dynamic_demands(graph: &SourceGraph) -> Vec<crate::DynamicImportDemand> {
        (0..graph.edges().len())
            .filter_map(|index| graph.dynamic_import_demand(index))
            .collect()
    }

    #[test]
    fn dynamic_selected_routes_preserve_aliases_barrels_and_original_evaluation() {
        let graph = SourceGraph::from_owned([
            input(
                "entry.mjs",
                "const {chosen, other, absent} = await import('barrel.mjs'); const value = (await import('barrel.mjs')).chosen;",
            ),
            input(
                "barrel.mjs",
                "console.log('barrel'); export {alias as chosen, alias as other} from 'middle.mjs';",
            ),
            input("middle.mjs", "export {value as alias} from 'leaf.mjs';"),
            input("leaf.mjs", "export const value = 1; export const unused = 2;"),
        ]);
        graph.validate_links().unwrap();
        let demands = dynamic_demands(&graph);
        assert_eq!(demands.len(), 2);
        assert_ne!(
            demands[0].evaluation.dependency,
            demands[1].evaluation.dependency
        );
        assert_ne!(demands[0].source_edge, demands[1].source_edge);
        for demand in &demands {
            let original = &graph.edges()[demand.source_edge];
            assert_eq!(original.purpose, EdgePurpose::Execution);
            assert_eq!(original.to, GraphNode::Module(1));
            assert_eq!(demand.evaluation.to, GraphNode::Module(1));
            assert_eq!(demand.evaluation.purpose, EdgePurpose::Evaluation);
            let bindings = demand.bindings.as_ref().unwrap();
            let names = if demand.source_edge == demands[0].source_edge {
                vec!["chosen", "other"]
            } else {
                vec!["chosen"]
            };
            for name in names {
                let route: Vec<_> = bindings
                    .iter()
                    .filter(|edge| edge.imported == Some(ImportedName::Name(name.into())))
                    .map(|edge| edge.to)
                    .collect();
                assert_eq!(
                    route,
                    graph
                        .resolve_export(1, name, crate::Namespace::Value)
                        .unwrap()
                        .route
                );
                assert!(route.contains(&GraphNode::Segment(SegmentKey::new(1, 1))));
                assert!(route.contains(&GraphNode::Segment(SegmentKey::new(2, 0))));
                assert!(route.contains(&GraphNode::Segment(SegmentKey::new(3, 0))));
                assert!(!route.contains(&GraphNode::Segment(SegmentKey::new(3, 1))));
            }
            assert_eq!(
                bindings.len(),
                if demand.source_edge == demands[0].source_edge {
                    6
                } else {
                    3
                }
            );
            for edge in bindings {
                assert_eq!(edge.from, original.from);
                assert_eq!(edge.site, original.site);
                assert_eq!(edge.dependency, original.dependency);
                assert_eq!(edge.enclosing_function, original.enclosing_function);
                assert_eq!(edge.activation, Activation::Asynchronous);
                assert_eq!(edge.kind, EdgeKind::DynamicImport);
                assert_eq!(edge.purpose, EdgePurpose::Binding);
            }
        }
        // Demand derivation must not expose extra async edges to today's planner.
        assert_eq!(
            graph
                .edges()
                .iter()
                .filter(|e| e.kind == EdgeKind::DynamicImport)
                .count(),
            2
        );
    }

    #[test]
    fn dynamic_missing_or_unused_properties_need_only_original_evaluation() {
        for source in [
            "const {absent} = await import('leaf.mjs');",
            "await import('leaf.mjs');",
            "import('leaf.mjs');",
            "const unused = await import('leaf.mjs');",
            "import('leaf.mjs').then(ns => ns.absent);",
        ] {
            let graph = SourceGraph::from_owned([
                input("entry.mjs", source),
                input("leaf.mjs", "export const value = 1;"),
            ]);
            graph.validate_links().unwrap();
            let demands = dynamic_demands(&graph);
            assert_eq!(demands.len(), 1, "{source}");
            assert_eq!(demands[0].bindings, Some(Vec::new()), "{source}");
            assert_eq!(demands[0].evaluation.to, GraphNode::Module(1));
        }
        let graph = SourceGraph::from_owned([
            input("entry.mjs", "import {absent} from 'leaf.mjs';"),
            input("leaf.mjs", "export const value = 1;"),
        ]);
        assert!(graph.validate_links().is_err());
        assert!(dynamic_demands(&graph).is_empty());
    }

    #[test]
    fn dynamic_unknown_reflected_escaping_and_receiver_namespaces_stay_full() {
        for source in [
            "const ns = await import('leaf.mjs'); consume(ns);",
            "const ns = await import('leaf.mjs'); Object.keys(ns);",
            "const ns = await import('leaf.mjs'); ns[key];",
            "export const ns = await import('leaf.mjs');",
            "const ns = await import('leaf.mjs'); ns.fn();",
            "(await import('leaf.mjs')).fn();",
            "import('leaf.mjs').then(ns => ns.fn());",
        ] {
            let graph = SourceGraph::from_owned([
                input("entry.mjs", source),
                input("leaf.mjs", "export function fn() { return this; }"),
            ]);
            graph.validate_links().unwrap();
            assert_eq!(dynamic_demands(&graph)[0].bindings, None, "{source}");
        }
    }

    #[test]
    fn dynamic_possible_then_assimilation_stays_full_even_without_property_reads() {
        for exports in [
            "export function then(resolve) { resolve(1); } export const value = 1;",
            "export {handler as then} from 'handler.mjs'; export const value = 1;",
            "export * from 'handler.mjs';",
            // Non-callability is intentionally not inferred in structural linking.
            "export const then = 1; export const value = 1;",
        ] {
            for source in [
                "const {value} = await import('leaf.mjs');",
                "import('leaf.mjs');",
            ] {
                let graph = SourceGraph::from_owned([
                    input("entry.mjs", source),
                    input("leaf.mjs", exports),
                    input(
                        "handler.mjs",
                        "export function handler() {} export {handler as then};",
                    ),
                ]);
                graph.validate_links().unwrap();
                assert_eq!(dynamic_demands(&graph)[0].bindings, None, "{exports}");
            }
        }
    }

    #[test]
    fn dynamic_opaque_external_and_context_routes_stay_full() {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            "exports.value = 1;",
            Path::new("opaque.cjs"),
            ModuleFormat::CommonJs,
        )
        .unwrap();
        let cjs =
            OwnedSourceFileInput::from_parsed("opaque.cjs".into(), parsed, Default::default());
        let graph = SourceGraph::from_owned([
            input("entry.mjs", "const {value} = await import('opaque.cjs');"),
            cjs,
        ]);
        graph.validate_links().unwrap();
        assert_eq!(dynamic_demands(&graph)[0].bindings, None);

        for barrel in [
            "export {value} from 'external';",
            "export * from 'external';",
        ] {
            let mut target = input("barrel.mjs", barrel);
            let dependency = target.facts.dependencies().get(0).unwrap().id();
            target
                .resolutions
                .insert(dependency, DependencyTarget::External("external".into()));
            let graph = SourceGraph::from_owned([
                input("entry.mjs", "const {value} = await import('barrel.mjs');"),
                target,
            ]);
            graph.validate_links().unwrap();
            assert_eq!(dynamic_demands(&graph)[0].bindings, None);
        }

        let mut entry = input("entry.mjs", "const {value} = await import('external');");
        let dependency = entry.facts.dependencies().get(0).unwrap().id();
        entry
            .resolutions
            .insert(dependency, DependencyTarget::External("external".into()));
        let graph = SourceGraph::from_owned([entry]);
        graph.validate_links().unwrap();
        assert_eq!(dynamic_demands(&graph)[0].bindings, None);
        assert!(matches!(
            dynamic_demands(&graph)[0].evaluation.to,
            GraphNode::Unlinked { .. }
        ));

        let mut entry = input(
            "entry.mjs",
            "const {value} = await import(`./${name}.mjs`);",
        );
        let dependency = entry.facts.dependencies().get(0).unwrap().id();
        entry.resolutions.insert(
            dependency,
            DependencyTarget::Context(BTreeMap::from([("./leaf.mjs".into(), "leaf.mjs".into())])),
        );
        let graph = SourceGraph::from_owned([entry, input("leaf.mjs", "export const value = 1;")]);
        graph.validate_links().unwrap();
        assert_eq!(dynamic_demands(&graph)[0].bindings, None);
    }

    #[test]
    fn dynamic_routes_use_borrowed_cache_observations_and_current_resolution() {
        use segment_cache::{CacheRecord, ParserConfig};

        let sources = [
            ("entry.mjs", "async function load() { const {chosen} = await import('barrel.mjs'); return chosen; }"),
            ("barrel.mjs", "import {value as local} from 'leaf.mjs'; export {local as chosen};"),
            ("leaf.mjs", "export const value = 1;"),
        ];
        let cold = SourceGraph::from_owned(sources.map(|(path, source)| input(path, source)));
        let mut warm = SourceGraph::from_owned(sources.map(|(path, source)| {
            let mut input = input(path, source);
            let parsed = parse_module(
                &logger::StdioLogger::new(),
                source,
                Path::new(path),
                ModuleFormat::Esm,
            )
            .unwrap();
            let bytes = CacheRecord::from_parsed(source, &parsed)
                .unwrap()
                .encode(source)
                .unwrap();
            drop(parsed);
            let record = CacheRecord::decode(
                &bytes,
                source,
                ParserConfig::for_path(Path::new(path), ModuleFormat::Esm).unwrap(),
            )
            .unwrap();
            input = OwnedSourceFileInput::from_parsed(
                path.into(),
                record.to_parsed(source).unwrap(),
                input.resolutions,
            );
            input
        }));
        cold.validate_links().unwrap();
        warm.validate_links().unwrap();
        assert_eq!(dynamic_demands(&cold), dynamic_demands(&warm));
        let demands = dynamic_demands(&warm);
        assert!(demands[0].evaluation.enclosing_function.is_some());
        let route: Vec<_> = demands[0]
            .bindings
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.to)
            .collect();
        assert!(route.contains(&GraphNode::Segment(SegmentKey::new(1, 1))));
        assert!(route.contains(&GraphNode::Segment(SegmentKey::new(2, 0))));

        warm.patch_owned(
            Path::new("leaf.mjs"),
            input(
                "leaf.mjs",
                "export const value = 1; export function then() {}",
            ),
        )
        .unwrap();
        // An unrelated leaf `then` is not exported by the original barrel.
        assert!(dynamic_demands(&warm)[0].bindings.is_some());
        warm.patch_owned(
            Path::new("barrel.mjs"),
            input(
                "barrel.mjs",
                "export * from 'leaf.mjs'; export {value as chosen} from 'leaf.mjs';",
            ),
        )
        .unwrap();
        warm.validate_links().unwrap();
        assert_eq!(dynamic_demands(&warm)[0].bindings, None);
    }

    #[test]
    fn contexts_expand_real_occurrences_and_reject_mismatched_resolutions() {
        let mut main = input(
            "main.mjs",
            "export const load = name => import(`./parts/${name}.mjs`);",
        );
        let dependency = main.facts.dependencies().get(0).unwrap().id();
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
            main.facts.dependencies().get(0).unwrap().id(),
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
            .facts()
            .dependencies()
            .iter()
            .map(|d| (d.id(), DependencyTarget::File("empty.ts".into())))
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
