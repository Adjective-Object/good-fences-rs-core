//! Lossy tracker inputs are projected once into the shared structural edges.
use crate::*;
use ast_segmenter::{
    owned::{DependencyId, ImportedName},
    raw_module_deps::Symbol,
    ImportTarget,
};

impl SourceGraph {
    pub(crate) fn legacy_edges(&self, file_id: u32) -> Vec<OccurrenceEdge> {
        let file = &self.files[file_id as usize];
        let mut edges = Vec::new();
        for (index, segment) in file.segments.iter().enumerate() {
            let from = GraphNode::Segment(SegmentKey::new(file_id, index as u32));
            let base = OccurrenceEdge {
                from,
                to: from,
                kind: EdgeKind::LocalReference,
                purpose: EdgePurpose::Binding,
                activation: Activation::Synchronous,
                type_only: false,
                site: segment.span,
                dependency: None,
                enclosing_function: None,
                imported: None,
                binding: None,
                read: true,
                write: false,
            };
            for (name, type_only) in segment.variables.get_escaped_symbols_with_type_only() {
                if let Some(target) =
                    self.resolve_symbol_in_file(file_id, name.as_ref(), index as u32)
                {
                    edges.push(OccurrenceEdge {
                        to: GraphNode::Segment(target),
                        type_only,
                        ..base.clone()
                    });
                }
            }
            let mut import = |specifier: &str,
                              symbol: Option<ImportedName>,
                              type_only: bool,
                              kind: EdgeKind,
                              purpose: EdgePurpose| {
                let target = file
                    .resolved_import_paths
                    .get(specifier)
                    .and_then(|p| self.file_id(p))
                    .map(GraphNode::Module)
                    // Legacy inputs have no occurrence IDs or external/missing distinction.
                    .unwrap_or(GraphNode::Unlinked {
                        file_id,
                        dependency: DependencyId(segment.span.start),
                    });
                edges.push(OccurrenceEdge {
                    to: target,
                    kind,
                    purpose,
                    type_only,
                    imported: symbol,
                    activation: if kind == EdgeKind::DynamicImport {
                        Activation::Asynchronous
                    } else {
                        Activation::Synchronous
                    },
                    ..base.clone()
                });
            };
            for (specifier, symbols) in &segment.module_deps.imports {
                for symbol in symbols {
                    import(
                        specifier,
                        Some(imported(&symbol.symbol)),
                        symbol.tags.is_type_only,
                        EdgeKind::Import,
                        EdgePurpose::Binding,
                    );
                }
            }
            for (specifier, symbols) in &segment.module_deps.dynamic_imports {
                for symbol in symbols {
                    import(
                        specifier,
                        Some(imported(symbol)),
                        false,
                        EdgeKind::DynamicImport,
                        EdgePurpose::Binding,
                    );
                }
            }
            for specifier in &segment.module_deps.requires {
                import(
                    specifier,
                    None,
                    false,
                    EdgeKind::Require,
                    EdgePurpose::Execution,
                );
            }
            for specifier in &segment.module_deps.executed_paths {
                import(
                    specifier,
                    None,
                    false,
                    EdgeKind::Import,
                    EdgePurpose::Execution,
                );
            }
            for (specifier, exports) in &segment.module_deps.exports_from {
                for export in exports {
                    let symbol = match &export.imported_as {
                        ImportTarget::Namespace => ImportedName::Namespace,
                        ImportTarget::ExportedSymbol(name) => {
                            ImportedName::Name(exported_name(name).to_owned())
                        }
                    };
                    import(
                        specifier,
                        Some(symbol),
                        export.tags.is_type_only,
                        EdgeKind::ReExport,
                        EdgePurpose::Binding,
                    );
                }
            }
        }
        edges
    }
}

fn imported(symbol: &Symbol) -> ImportedName {
    match symbol {
        Symbol::Named(name) => ImportedName::Name(name.to_string()),
        Symbol::Default => ImportedName::Name("default".to_owned()),
        Symbol::Namespace => ImportedName::Namespace,
    }
}
