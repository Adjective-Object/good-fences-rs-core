use ahashmap::AHashSet;
use ast_segmenter::owned::{BindingId, ExportValue, ImportedName};
use ast_segmenter::{ExportedSymbol, ImportTarget};

use crate::{DependencyTarget, GraphNode, SegmentKey, SourceGraph};

pub(crate) fn exported_name(symbol: &ExportedSymbol) -> &str {
    match symbol {
        ExportedSymbol::Named(name) => name.as_ref(),
        ExportedSymbol::Default => "default",
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDiagnostic {
    pub file_id: u32,
    pub site: oxc_span::Span,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    Value,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExportIdentity {
    CommonJs(u32, String),
    Binding(u32, BindingId),
    Expression(SegmentKey),
    Namespace(u32),
    External(String, Option<String>),
    /// Ambiguity involving unknown external star surfaces belongs to native linking.
    Native(u32, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedExport {
    pub identity: ExportIdentity,
    pub declaration: GraphNode,
    /// Includes intermediate barrels, independently of the final binding identity.
    pub route: Vec<GraphNode>,
}

enum Candidate {
    Local(LinkedExport),
    Indirect(u32, String, GraphNode),
}

impl SourceGraph {
    pub fn diagnostics(&self) -> &[LinkDiagnostic] {
        &self.diagnostics
    }

    pub fn validate_links(&self) -> eyre::Result<()> {
        eyre::ensure!(
            self.diagnostics.is_empty(),
            "link diagnostics: {:?}",
            self.diagnostics
        );
        Ok(())
    }

    /// Representative declaration; checked export routes include every declaration.
    pub fn resolve_binding(&self, file_id: u32, id: BindingId) -> Option<GraphNode> {
        let input = self.files.get(file_id as usize)?.owned.as_ref()?;
        let binding = input.facts.bindings.iter().find(|b| b.id == id)?;
        Some(self.segment_at(file_id, *binding.declarations.first()?))
    }

    /// Fails on missing or ambiguous bindings; a cycle alone does not define an export.
    pub fn resolve_export(
        &self,
        file: u32,
        name: &str,
        namespace: Namespace,
    ) -> eyre::Result<LinkedExport> {
        self.resolve_optional_export(file, name, namespace)?
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing {:?} export {name:?} in {:?} (including unresolved cycles)",
                    namespace,
                    self.file_path(file)
                )
            })
    }

    /// Namespace property reads may name an absent export, but not an ambiguous one.
    pub fn resolve_optional_export(
        &self,
        file: u32,
        name: &str,
        namespace: Namespace,
    ) -> eyre::Result<Option<LinkedExport>> {
        let mut active = AHashSet::default();
        let results = self.export_candidates(file, name, namespace, &mut active);
        let Some(mut result) = results.first().cloned() else {
            return Ok(None);
        };
        let native = namespace == Namespace::Value
            && results.iter().any(|candidate| {
                matches!(
                    candidate.identity,
                    ExportIdentity::External(..) | ExportIdentity::Native(..)
                )
            })
            && results
                .iter()
                .any(|candidate| candidate.identity != result.identity);
        for candidate in results.iter().skip(1) {
            eyre::ensure!(
                native || candidate.identity == result.identity,
                "ambiguous {:?} export {name:?} in {:?}",
                namespace,
                self.file_path(file)
            );
            for node in &candidate.route {
                if !result.route.contains(node) {
                    result.route.push(*node);
                }
                if native {
                    result.identity = ExportIdentity::Native(file, name.to_owned());
                }
            }
        }
        let mut seen = AHashSet::default();
        result.route.retain(|node| seen.insert(*node));
        Ok(Some(result))
    }

    pub fn has_external_star_exports(&self, file: u32) -> bool {
        let mut pending = vec![file];
        let mut seen = AHashSet::default();
        while let Some(file) = pending.pop() {
            if !seen.insert(file) {
                continue;
            }
            for export in self
                .file_facts(file)
                .into_iter()
                .flat_map(|facts| &facts.exports)
                .filter(|export| !export.type_only && export.name.is_none())
            {
                if let ExportValue::ReExport { dependency, .. } = export.value {
                    match self.dependency_target(file, dependency) {
                        Some(DependencyTarget::External(_)) => return true,
                        Some(DependencyTarget::File(path)) => pending.extend(self.file_id(path)),
                        _ => {}
                    }
                }
            }
        }
        false
    }

    /// Preserve concrete candidate demands when unknown external stars defer linking.
    pub fn resolve_export_alternatives(
        &self,
        file: u32,
        name: &str,
        namespace: Namespace,
    ) -> eyre::Result<Vec<LinkedExport>> {
        let linked = self.resolve_export(file, name, namespace)?;
        if matches!(linked.identity, ExportIdentity::Native(..)) {
            Ok(self.export_candidates(file, name, namespace, &mut AHashSet::default()))
        } else {
            Ok(vec![linked])
        }
    }

    fn export_candidates(
        &self,
        file_id: u32,
        name: &str,
        namespace: Namespace,
        active: &mut AHashSet<(u32, String, Namespace)>,
    ) -> Vec<LinkedExport> {
        let key = (file_id, name.to_owned(), namespace);
        if !active.insert(key.clone()) {
            return vec![];
        }
        let Some(file) = self.files.get(file_id as usize) else {
            active.remove(&key);
            return vec![];
        };
        if namespace == Namespace::Value
            && file
                .owned
                .as_ref()
                .is_some_and(|input| input.facts.commonjs)
        {
            active.remove(&key);
            return vec![LinkedExport {
                identity: ExportIdentity::CommonJs(file_id, name.to_owned()),
                declaration: GraphNode::Module(file_id),
                route: vec![GraphNode::Module(file_id)],
            }];
        }
        let mut explicit = Vec::new();
        let mut stars = Vec::new();
        let mut has_explicit = false;
        if let Some(input) = &file.owned {
            for export in &input.facts.exports {
                if namespace == Namespace::Value && export.type_only {
                    continue;
                }
                let node = self.segment_at(file_id, export.span);
                let is_explicit = export.name.as_deref() == Some(name);
                if !is_explicit && export.name.is_some() {
                    continue;
                }
                has_explicit |= is_explicit;
                match &export.value {
                    ExportValue::Binding(binding) if is_explicit => {
                        if let Some(import) =
                            input.facts.imports.iter().find(|i| i.binding == *binding)
                        {
                            if let Some(candidate) = self.resolved_candidate(
                                file_id,
                                import.dependency,
                                &import.imported,
                                node,
                            ) {
                                explicit.push(candidate);
                            }
                        } else if let Some(declaration) = self.resolve_binding(file_id, *binding) {
                            explicit.push(Candidate::Local(LinkedExport {
                                identity: ExportIdentity::Binding(file_id, *binding),
                                declaration,
                                route: std::iter::once(node)
                                    .chain(
                                        input
                                            .facts
                                            .bindings
                                            .iter()
                                            .filter(|b| b.id == *binding)
                                            .flat_map(|b| &b.declarations)
                                            .map(|&span| self.segment_at(file_id, span)),
                                    )
                                    .collect(),
                            }));
                        }
                    }
                    ExportValue::Expression(_) if is_explicit => {
                        if let GraphNode::Segment(segment) = node {
                            explicit.push(Candidate::Local(LinkedExport {
                                identity: ExportIdentity::Expression(segment),
                                declaration: node,
                                route: vec![node],
                            }));
                        }
                    }
                    ExportValue::ReExport {
                        dependency,
                        imported,
                    } => {
                        if is_explicit {
                            if let Some(candidate) =
                                self.resolved_candidate(file_id, *dependency, imported, node)
                            {
                                explicit.push(candidate);
                            }
                        } else if name != "default" {
                            if let Some(candidate) = self.resolved_candidate(
                                file_id,
                                *dependency,
                                &ImportedName::Name(name.to_owned()),
                                node,
                            ) {
                                stars.push(candidate);
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            let symbol = ExportedSymbol::from(name);
            for (index, segment) in file.segments.iter().enumerate() {
                let node = GraphNode::Segment(SegmentKey::new(file_id, index as u32));
                if let Some(local) = segment.module_deps.exports_locals.get(&symbol) {
                    if namespace != Namespace::Value || !local.tags.is_type_only {
                        has_explicit = true;
                        explicit.push(Candidate::Local(LinkedExport {
                            identity: ExportIdentity::Expression(SegmentKey::new(
                                file_id,
                                index as u32,
                            )),
                            declaration: node,
                            route: vec![node],
                        }));
                    }
                }
                for (specifier, exports) in &segment.module_deps.exports_from {
                    for export in exports {
                        if namespace == Namespace::Value && export.tags.is_type_only {
                            continue;
                        }
                        let exported =
                            export
                                .exported_as
                                .as_ref()
                                .or_else(|| match &export.imported_as {
                                    ImportTarget::ExportedSymbol(name) => Some(name),
                                    _ => None,
                                });
                        let is_explicit = exported == Some(&symbol);
                        has_explicit |= is_explicit;
                        let Some(target) = file
                            .resolved_import_paths
                            .get(specifier)
                            .and_then(|p| self.file_id(p))
                        else {
                            continue;
                        };
                        if is_explicit {
                            let imported = match &export.imported_as {
                                ImportTarget::ExportedSymbol(s) => {
                                    ImportedName::Name(exported_name(s).to_owned())
                                }
                                ImportTarget::Namespace => ImportedName::Namespace,
                            };
                            explicit.push(self.import_candidate(target, &imported, node));
                        } else if exported.is_none() && name != "default" {
                            stars.push(Candidate::Indirect(target, name.to_owned(), node));
                        }
                    }
                }
            }
        }
        let mut results = Vec::new();
        for candidate in if has_explicit { explicit } else { stars } {
            match candidate {
                Candidate::Local(result) => results.push(result),
                Candidate::Indirect(target, imported, node) => {
                    for mut result in self.export_candidates(target, &imported, namespace, active) {
                        result.route.insert(0, node);
                        results.push(result);
                    }
                }
            }
        }
        active.remove(&key);
        results
    }

    fn import_candidate(&self, target: u32, imported: &ImportedName, node: GraphNode) -> Candidate {
        match imported {
            ImportedName::Name(name) => Candidate::Indirect(target, name.clone(), node),
            ImportedName::Namespace => Candidate::Local(LinkedExport {
                identity: ExportIdentity::Namespace(target),
                declaration: GraphNode::Module(target),
                route: vec![node, GraphNode::Module(target)],
            }),
        }
    }
    fn resolved_candidate(
        &self,
        file_id: u32,
        dependency: ast_segmenter::owned::DependencyId,
        imported: &ImportedName,
        node: GraphNode,
    ) -> Option<Candidate> {
        match self.dependency_target(file_id, dependency)? {
            DependencyTarget::File(path) => {
                Some(self.import_candidate(self.file_id(path)?, imported, node))
            }
            DependencyTarget::External(specifier) => {
                let declaration = GraphNode::Unlinked {
                    file_id,
                    dependency,
                };
                Some(Candidate::Local(LinkedExport {
                    identity: ExportIdentity::External(
                        specifier.clone(),
                        match imported {
                            ImportedName::Name(name) => Some(name.clone()),
                            ImportedName::Namespace => None,
                        },
                    ),
                    declaration,
                    route: vec![node, declaration],
                }))
            }
            DependencyTarget::Missing(_) | DependencyTarget::Context(_) => None,
        }
    }
}
