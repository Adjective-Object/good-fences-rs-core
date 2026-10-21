use ast_segmenter::owned::ImportedName;

use crate::{
    Activation, DependencyTarget, EdgeKind, EdgePurpose, ExportIdentity, GraphNode, Namespace,
    OccurrenceEdge, SourceGraph,
};

/// Opt-in structural demands for one original import occurrence.
///
/// These are not additional graph edges. Consumers must model separate export
/// roots and original-module evaluation before narrowing namespace demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicImportDemand {
    /// Index in `SourceGraph::edges`, valid only at the current graph revision.
    pub source_edge: usize,
    /// Original target evaluation, never the canonical export's leaf module.
    /// Every selected binding requires this obligation at the same occurrence.
    pub evaluation: OccurrenceEdge,
    /// `None` requires the full namespace. `Some` contains checked binding routes;
    /// an empty vector needs only evaluation (including absent property reads).
    /// Group routes by `imported`, not by target or canonical binding identity.
    pub bindings: Option<Vec<OccurrenceEdge>>,
}

impl SourceGraph {
    /// Derive asynchronous binding routes without changing retention or scheduling.
    ///
    /// Missing properties need no binding and are not static link errors. Unknown
    /// surfaces and unsafe observation shapes retain the original namespace.
    pub fn dynamic_import_demand(&self, source_edge: usize) -> Option<DynamicImportDemand> {
        let occurrence = self.edges().get(source_edge)?;
        if occurrence.kind != EdgeKind::DynamicImport
            || occurrence.activation != Activation::Asynchronous
            || occurrence.type_only
            || occurrence.purpose == EdgePurpose::Binding
        {
            return None;
        }
        Some(DynamicImportDemand {
            source_edge,
            evaluation: OccurrenceEdge {
                purpose: EdgePurpose::Evaluation,
                ..occurrence.clone()
            },
            bindings: self.dynamic_binding_routes(occurrence),
        })
    }

    fn dynamic_binding_routes(&self, occurrence: &OccurrenceEdge) -> Option<Vec<OccurrenceEdge>> {
        let file = match occurrence.from {
            GraphNode::Module(file) | GraphNode::Unlinked { file_id: file, .. } => file,
            GraphNode::Segment(key) => key.file_id,
        };
        let dependency = occurrence.dependency?;
        if !matches!(
            self.dependency_target(file, dependency),
            Some(DependencyTarget::File(_))
        ) {
            return None;
        }
        let GraphNode::Module(target) = occurrence.to else {
            return None;
        };
        if self.file_facts(target)?.commonjs() || self.has_external_star_exports(target) {
            return None;
        }
        // Even an unobserved import assimilates a callable namespace `then`.
        // We do not prove callability here, so any possible export blocks narrowing.
        if self
            .resolve_optional_export(target, "then", Namespace::Value)
            .ok()?
            .is_some()
        {
            return None;
        }
        let observation = self
            .file_facts(file)?
            .emission()
            .dynamic_namespaces()
            .iter()
            .find(|observation| observation.dependency() == dependency)?;
        let properties = observation.properties()?;
        let mut bindings = Vec::new();
        for (name, receiver) in properties {
            // A call can expose the namespace through `this`, including via a
            // re-exported function. No receiver-safety proof is available here.
            if receiver {
                return None;
            }
            let Some(export) = self
                .resolve_optional_export(target, name, Namespace::Value)
                .ok()?
            else {
                continue;
            };
            if !matches!(
                export.identity,
                ExportIdentity::Binding(..) | ExportIdentity::Expression(..)
            ) {
                return None;
            }
            for to in export.route {
                let route_file = match to {
                    GraphNode::Module(file) => file,
                    GraphNode::Segment(key) => key.file_id,
                    GraphNode::Unlinked { .. } => return None,
                };
                if self.file_facts(route_file)?.commonjs() {
                    return None;
                }
                bindings.push(OccurrenceEdge {
                    to,
                    purpose: EdgePurpose::Binding,
                    imported: Some(ImportedName::Name(name.to_owned())),
                    ..occurrence.clone()
                });
            }
        }
        Some(bindings)
    }
}
