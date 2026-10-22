//! Explicit runtime authorization and build-local, all-reference certification.
//! Certified overlays replace graph obligations without changing cached facts.
use std::{collections::HashSet, path::PathBuf};

use ast_segmenter::{
    emission::LazyWrapperKind,
    owned::{BindingId, DependencyId},
    views::{DeferredForm, ExportValue, ImportedName, LazyModuleCandidate, SourceSlot},
};
use oxc_span::Span;
use sha2::{Digest, Sha256};
use source_graph::{
    DependencyTarget, EdgeKind, ExportIdentity, GraphNode, GraphRevision, LinkedExport, Namespace,
    SegmentKey, SourceGraph,
};

use crate::policy::EffectPolicy;

/// An exact graph module/export identity, not a source spelling or package pattern.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConstructorExport {
    pub module: PathBuf,
    pub export: String,
}

/// Opting in authorizes removable/synthesizable LazyModule allocations with
/// split-specific warming state and no initializer when options are absent.
/// Ordinary library methods and LazyImport/LazyAction wrappers are not replaced:
/// selection caches, governance, loaded-state reads, action options and arguments
/// remain the actual registered library's responsibility.
/// Registered wrapper allocations are removable value initializers, not
/// independent source effects; their genuine argument execution is never waived.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LazyRuntime {
    pub module: ConstructorExport,
    pub import: ConstructorExport,
    pub action: ConstructorExport,
}

pub fn configuration_fingerprint(runtimes: &[LazyRuntime]) -> [u8; 32] {
    let mut canonical = runtimes.to_vec();
    canonical.sort();
    canonical.dedup();
    Sha256::digest(format!("lazy-runtime-wiring-v2\0{canonical:?}").as_bytes()).into()
}

/// Original recipe coordinates and complete genuine input dependencies. Effects
/// must be evaluated once at this original site, never copied per synthesized instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedSlot {
    pub span: Span,
    pub operations: Vec<u32>,
    pub references: Vec<u32>,
    pub effects: Vec<u32>,
    pub invocations: Vec<u32>,
    pub dependencies: Vec<DependencyId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSelector {
    pub candidate: u32,
    pub owner: SegmentKey,
    pub construction: Span,
    pub constructor_reference: u32,
    pub constructor: ExportIdentity,
    /// Determined from canonical identities, never the cached spelling hint.
    pub kind: LazyWrapperKind,
    pub value_use: ast_segmenter::invocation::ValueUse,
    /// A nested initializer's value demand belongs to this binding, not to the
    /// enclosing function/statement's independently demanded value.
    pub nested_result: Option<BindingId>,
    /// Reads of the exact nested result binding protect its initializer,
    /// including captures whose source precedes the initializer.
    pub nested_result_read: bool,
    pub module_reference: u32,
    pub property: String,
    pub selected: LinkedExport,
    /// Includes module/getter and every wrapper-specific argument, unchanged.
    pub arguments: Vec<PreservedSlot>,
}

impl CertifiedSelector {
    pub(crate) fn value_demanded(&self, statement_value: bool) -> bool {
        match self.value_use {
            ast_segmenter::invocation::ValueUse::Required => true,
            ast_segmenter::invocation::ValueUse::StatementValue => {
                if self.nested_result.is_some() {
                    self.nested_result_read
                } else {
                    statement_value
                }
            }
            ast_segmenter::invocation::ValueUse::Discarded => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedModule {
    pub candidate: u32,
    pub owner: SegmentKey,
    pub binding: BindingId,
    pub declaration: Span,
    pub construction: Span,
    pub constructor_reference: u32,
    pub constructor: ExportIdentity,
    pub dependency: DependencyId,
    pub import_span: Span,
    pub source_edge: usize,
    /// Original barrel evaluation, independently of selected canonical leaves.
    pub target: u32,
    pub callback: PreservedSlot,
    pub options: Option<PreservedSlot>,
    pub selectors: Vec<CertifiedSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certification {
    revision: GraphRevision,
    policy_fingerprint: [u8; 32],
    configuration_fingerprint: [u8; 32],
    modules: Vec<CertifiedModule>,
}

impl Certification {
    pub fn modules(&self) -> &[CertifiedModule] {
        &self.modules
    }

    pub(crate) fn consumes_binding(&self, file: u32, binding: BindingId) -> bool {
        self.modules
            .iter()
            .any(|module| module.owner.file_id == file && module.binding == binding)
    }

    pub(crate) fn consumes_edge(&self, graph: &SourceGraph, index: usize) -> bool {
        let edge = &graph.edges()[index];
        self.modules.iter().any(|module| {
            edge.from == GraphNode::Segment(module.owner)
                && (edge.kind == EdgeKind::DynamicImport
                    && edge.dependency == Some(module.dependency)
                    || graph
                        .file_facts(module.owner.file_id)
                        .and_then(|facts| {
                            facts
                                .references()
                                .get(module.constructor_reference as usize)
                        })
                        .is_some_and(|reference| {
                            edge.site == reference.span() && edge.binding == reference.binding()
                        }))
                || module.selectors.iter().any(|selector| {
                    edge.from == GraphNode::Segment(selector.owner)
                        && edge.binding == Some(module.binding)
                        && graph
                            .file_facts(selector.owner.file_id)
                            .is_some_and(|facts| {
                                facts
                                    .references()
                                    .get(selector.module_reference as usize)
                                    .is_some_and(|reference| reference.span() == edge.site)
                            })
                })
        })
    }

    pub(crate) fn affects(&self, key: SegmentKey) -> bool {
        self.modules.iter().any(|module| {
            module.owner == key
                || module
                    .selectors
                    .iter()
                    .any(|selector| selector.owner == key)
        })
    }

    pub(crate) fn allocation(&self, key: SegmentKey, span: Span) -> bool {
        self.modules.iter().any(|module| {
            module.owner == key && module.construction == span
                || module
                    .selectors
                    .iter()
                    .any(|selector| selector.owner == key && selector.construction == span)
        })
    }

    pub(crate) fn consumed_constructor(&self, key: SegmentKey, reference: u32) -> bool {
        self.modules
            .iter()
            .any(|module| module.owner == key && module.constructor_reference == reference)
    }

    /// Registered allocations remain real values, but only residual argument
    /// execution is an unconditional source event. Forced-effect policy wins.
    pub(crate) fn residual_effectful(
        &self,
        key: SegmentKey,
        summary: &crate::effects::StatementEffects,
    ) -> bool {
        !summary.known_effects.is_empty()
            || summary.sites.iter().any(|site| {
                !matches!(
                    site.reason(),
                    ast_segmenter::effects::EffectReason::Invocation
                        | ast_segmenter::effects::EffectReason::Construction
                ) || !self.allocation(key, site.span())
            })
    }

    pub(crate) fn invocation_removable(&self, key: SegmentKey, span: Span, demanded: bool) -> bool {
        self.modules.iter().any(|module| {
            module.owner == key && module.construction == span
                || module.selectors.iter().any(|selector| {
                    selector.owner == key
                        && selector.construction == span
                        && !selector.value_demanded(demanded)
                })
        })
    }

    pub(crate) fn consumed_read(&self, graph: &SourceGraph, key: SegmentKey, span: Span) -> bool {
        let Some(facts) = graph.file_facts(key.file_id) else {
            return false;
        };
        self.modules.iter().any(|module| {
            let matches = |reference| {
                facts
                    .references()
                    .get(reference as usize)
                    .is_some_and(|reference| reference.span() == span)
            };
            module.owner == key && matches(module.constructor_reference)
                || module.selectors.iter().any(|selector| {
                    selector.owner == key
                        && (matches(selector.module_reference)
                            || matches(selector.constructor_reference))
                })
        })
    }

    pub fn is_current(
        &self,
        graph: &SourceGraph,
        policy: &EffectPolicy,
        runtimes: &[LazyRuntime],
    ) -> bool {
        self.has_provenance(graph, policy)
            && self.configuration_fingerprint == configuration_fingerprint(runtimes)
    }

    pub(crate) fn has_provenance(&self, graph: &SourceGraph, policy: &EffectPolicy) -> bool {
        self.revision == graph.revision() && self.policy_fingerprint == policy.fingerprint()
    }
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn preserve(graph: &SourceGraph, file: u32, slot: SourceSlot<'_>) -> PreservedSlot {
    let facts = graph.file_facts(file).expect("checked source");
    PreservedSlot {
        span: slot.span(),
        operations: slot.operations().iter().collect(),
        references: facts
            .references()
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.type_only() && contains(slot.span(), r.span()))
            .map(|(i, _)| i as u32)
            .collect(),
        effects: facts
            .effects()
            .iter()
            .enumerate()
            .filter(|(_, e)| contains(slot.span(), e.span()))
            .map(|(i, _)| i as u32)
            .collect(),
        invocations: facts
            .invocations()
            .iter()
            .enumerate()
            .filter(|(_, i)| contains(slot.span(), i.span()))
            .map(|(i, _)| i as u32)
            .collect(),
        dependencies: facts
            .dependencies()
            .iter()
            .filter(|d| contains(slot.span(), d.span()))
            .map(|d| d.id())
            .collect(),
    }
}

fn fixed_binding(graph: &SourceGraph, file: u32, binding: BindingId) -> bool {
    let Some(facts) = graph.file_facts(file) else {
        return false;
    };
    let Some(record) = facts.bindings().iter().find(|b| b.id() == binding) else {
        return false;
    };
    if facts.commonjs()
        || !record.top_level()
        || record.type_only()
        || record.ambient()
        || record.declarations().len() != 1
        || facts
            .references()
            .iter()
            .any(|r| r.binding() == Some(binding) && r.write() && !r.type_only())
    {
        return false;
    }
    // Imports/classes/functions may have immutable linked identities without
    // being const-variable declarations. Mutable variable aliases are not proof.
    !facts.emission().segments().iter().any(|s| {
        matches!(s.deferred(), DeferredForm::Variable { binding: id, .. } if id == binding)
            && !record.constant()
    })
}

fn canonical_binding(
    graph: &SourceGraph,
    file: u32,
    binding: BindingId,
    active: &mut HashSet<(u32, BindingId)>,
) -> Option<ExportIdentity> {
    if !active.insert((file, binding)) || !fixed_binding(graph, file, binding) {
        return None;
    }
    let facts = graph.file_facts(file)?;
    if let Some(import) = facts
        .imports()
        .iter()
        .find(|i| i.binding() == binding && !i.type_only())
    {
        let ImportedName::Name(name) = import.imported() else {
            return None;
        };
        let DependencyTarget::File(path) = graph.dependency_target(file, import.dependency())?
        else {
            return None;
        };
        let linked = graph
            .resolve_export(graph.file_id(path)?, name, Namespace::Value)
            .ok()?;
        return canonical_export(graph, linked.identity, active);
    }
    for recipe in facts.emission().segments() {
        if let DeferredForm::Variable {
            binding: id,
            initializer: Some(slot),
        } = recipe.deferred()
        {
            if id == binding {
                if let Some(reference) = facts
                    .references()
                    .iter()
                    .find(|r| !r.type_only() && r.read() && !r.write() && r.span() == slot.span())
                {
                    return canonical_binding(graph, file, reference.binding()?, active);
                }
            }
        }
    }
    Some(ExportIdentity::Binding(file, binding))
}

fn canonical_export(
    graph: &SourceGraph,
    identity: ExportIdentity,
    active: &mut HashSet<(u32, BindingId)>,
) -> Option<ExportIdentity> {
    match identity {
        ExportIdentity::Binding(file, binding) => canonical_binding(graph, file, binding, active),
        ExportIdentity::Expression(key) if !graph.file_facts(key.file_id)?.commonjs() => {
            Some(ExportIdentity::Expression(key))
        }
        _ => None,
    }
}

fn constructor(graph: &SourceGraph, file: u32, reference: u32) -> Option<ExportIdentity> {
    let binding = graph
        .file_facts(file)?
        .references()
        .get(reference as usize)?
        .binding()?;
    canonical_binding(graph, file, binding, &mut HashSet::new())
}

fn registered(graph: &SourceGraph, export: &ConstructorExport) -> Option<ExportIdentity> {
    let linked = graph
        .resolve_export(
            graph.file_id(&export.module)?,
            &export.export,
            Namespace::Value,
        )
        .ok()?;
    canonical_export(graph, linked.identity, &mut HashSet::new())
}

fn eligible(
    graph: &SourceGraph,
    file: u32,
    index: u32,
    module: LazyModuleCandidate<'_>,
    runtime: &[ExportIdentity; 3],
) -> Option<CertifiedModule> {
    let facts = graph.file_facts(file)?;
    if !fixed_binding(graph, file, module.binding())
        || constructor(graph, file, module.constructor())? != runtime[0]
        || module.arguments().len() > 2
        || (module.options().is_some() && module.options_properties().is_none())
        || facts.exports().iter().any(|e| {
            !e.type_only()
                && matches!(e.value(), ExportValue::Binding(id) if id == module.binding())
        })
    {
        return None;
    }
    let DependencyTarget::File(path) = graph.dependency_target(file, module.dependency())? else {
        return None;
    };
    let target = graph.file_id(path)?;
    if graph.file_facts(target)?.commonjs()
        || graph.has_external_star_exports(target)
        || graph
            .resolve_optional_export(target, "then", Namespace::Value)
            .ok()?
            .is_some()
    {
        return None;
    }
    let source_edge = graph.edges().iter().position(|e| {
        e.kind == EdgeKind::DynamicImport
            && !e.type_only
            && e.from == GraphNode::Segment(SegmentKey::new(file, module.segment()))
            && e.to == GraphNode::Module(target)
            && e.dependency == Some(module.dependency())
    })?;
    let mut selectors = Vec::new();
    for (reference, occurrence) in facts
        .references()
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.type_only() && r.binding() == Some(module.binding()))
    {
        if !occurrence.read() || occurrence.write() {
            return None;
        }
        let (candidate_index, candidate) = facts
            .emission()
            .lazy_selectors()
            .iter()
            .enumerate()
            .find(|(_, s)| {
                s.module_reference() == reference as u32 && s.module_binding() == module.binding()
            })?;
        let identity = constructor(graph, file, candidate.constructor())?;
        if contains(module.construction(), candidate.construction()) {
            return None;
        }
        let kind = if identity == runtime[1] {
            LazyWrapperKind::LazyImport
        } else if identity == runtime[2] {
            LazyWrapperKind::LazyAction
        } else {
            return None;
        };
        let call = facts
            .invocations()
            .iter()
            .find(|i| i.span() == candidate.construction() && i.construct())?;
        if call.arguments().iter().any(|a| a.spread())
            || (call.value_use() == ast_segmenter::invocation::ValueUse::StatementValue
                && call.deferred().is_some()
                && candidate.result_binding().is_none())
        {
            return None;
        }
        let selected = graph
            .resolve_export(target, candidate.property(), Namespace::Value)
            .ok()?;
        if !matches!(
            selected.identity,
            ExportIdentity::Binding(..) | ExportIdentity::Expression(..)
        ) || selected.route.iter().any(|node| {
            let file = match node {
                GraphNode::Module(f) => *f,
                GraphNode::Segment(s) => s.file_id,
                GraphNode::Unlinked { .. } => return true,
            };
            graph.file_facts(file).is_none_or(|f| f.commonjs())
                || graph.has_external_star_exports(file)
        }) {
            return None;
        }
        let nested_result = candidate.result_binding().filter(|result| {
            facts
                .bindings()
                .iter()
                .any(|binding| binding.id() == *result && !binding.top_level())
        });
        selectors.push(CertifiedSelector {
            candidate: candidate_index as u32,
            owner: SegmentKey::new(file, candidate.segment()),
            construction: candidate.construction(),
            constructor_reference: candidate.constructor(),
            constructor: identity,
            kind,
            value_use: call.value_use(),
            nested_result,
            nested_result_read: nested_result.is_some_and(|result| {
                facts.references().iter().any(|reference| {
                    !reference.type_only()
                        && reference.read()
                        && reference.binding() == Some(result)
                })
            }),
            module_reference: reference as u32,
            property: candidate.property().to_owned(),
            selected,
            arguments: candidate
                .arguments()
                .iter()
                .map(|s| preserve(graph, file, s))
                .collect(),
        });
    }
    if selectors.is_empty() {
        return None;
    }
    Some(CertifiedModule {
        candidate: index,
        owner: SegmentKey::new(file, module.segment()),
        binding: module.binding(),
        declaration: module.declaration(),
        construction: module.construction(),
        constructor_reference: module.constructor(),
        constructor: runtime[0].clone(),
        dependency: module.dependency(),
        import_span: module.import_span(),
        source_edge,
        target,
        callback: preserve(graph, file, module.callback()),
        options: module.options().map(|s| preserve(graph, file, s)),
        selectors,
    })
}

/// Certify before retention. Unsupported or mixed-use instances are absent in
/// their entirety; source-local cache facts remain untouched.
pub fn certify(
    graph: &SourceGraph,
    policy: &EffectPolicy,
    runtimes: &[LazyRuntime],
) -> eyre::Result<Certification> {
    graph.validate_links()?;
    let mut result = Certification {
        revision: graph.revision(),
        policy_fingerprint: policy.fingerprint(),
        configuration_fingerprint: configuration_fingerprint(runtimes),
        modules: Vec::new(),
    };
    // External-native linking keeps original clauses and source-module jobs.
    // It does not yet admit the lexical reconstruction needed for lazy wiring.
    if graph.module_ids().any(|file| {
        graph.file_facts(file).is_some_and(|facts| {
            facts.dependencies().iter().any(|dependency| {
                !dependency.type_only()
                    && matches!(
                        graph.dependency_target(file, dependency.id()),
                        Some(DependencyTarget::External(_))
                    )
            })
        })
    }) {
        return Ok(result);
    }
    let registered: Vec<_> = runtimes
        .iter()
        .filter_map(|r| {
            let identities = [
                registered(graph, &r.module)?,
                registered(graph, &r.import)?,
                registered(graph, &r.action)?,
            ];
            (identities[0] != identities[1]
                && identities[0] != identities[2]
                && identities[1] != identities[2])
                .then_some(identities)
        })
        .collect();
    let trusted: Vec<_> = registered
        .iter()
        .filter(|runtime| {
            !registered.iter().any(|other| {
                (0..3).any(|role| {
                    (0..3)
                        .any(|other_role| role != other_role && runtime[role] == other[other_role])
                })
            })
        })
        .collect();
    if trusted.is_empty() {
        return Ok(result);
    }
    for file in graph.module_ids() {
        let Some(facts) = graph.file_facts(file) else {
            continue;
        };
        if facts.commonjs() {
            continue;
        }
        for (index, candidate) in facts.emission().lazy_modules().iter().enumerate() {
            if let Some(module) = trusted
                .iter()
                .find_map(|r| eligible(graph, file, index as u32, candidate, r))
            {
                result.modules.push(module);
            }
        }
    }
    Ok(result)
}
