use std::{collections::BTreeMap, sync::Arc};

use ast_segmenter::owned::{DependencyKind, ModuleFacts};
use eyre::{ensure, OptionExt, WrapErr};
use source_graph::{DependencyTarget, ExportIdentity, Namespace, SegmentKey, SourceGraph};

use crate::{effects, policy::EffectPolicy};

pub(crate) fn analyze(
    graph: &SourceGraph,
    policy: &EffectPolicy,
) -> eyre::Result<BTreeMap<SegmentKey, bool>> {
    let mut analysis = ecco::Analysis::new();
    for file in graph.module_ids() {
        let facts = graph
            .file_facts(file)
            .ok_or_eyre("missing Ecco module facts")?;
        let Some(parsed) = &facts.ecco else {
            continue;
        };
        analysis.add_file(file, Arc::new(parsed.clone()))?;
    }
    let result = analysis.solve(&|file: &u32, reference: &ecco::ExternalReference| {
        resolve(graph, *file, reference).wrap_err_with(|| {
            format!(
                "{}: effect reference at bytes {}..{}",
                graph.file_path(*file).unwrap().display(),
                reference.span.start,
                reference.span.end
            )
        })
    })?;
    let mut effects = BTreeMap::new();
    // shortcut: deferred policy/TDZ barriers disable inferred pruning until Ecco
    // exposes invocation-specific policy and initialization-error dependencies.
    let structural_override = !policy.honors_pure_annotations()
        || graph.module_ids().any(|file| {
            graph.file_facts(file).is_some_and(|facts| {
                facts.commonjs
                    || crate::link::has_externals(graph, file)
                    || facts.invocations.iter().any(|call| {
                        call.deferred.is_some() && policy.matching_patterns(call).next().is_some()
                    })
                    || facts.effects.iter().any(|site| {
                        let Some(deferred) = site.deferred else {
                            return false;
                        };
                        if site.reason != ast_segmenter::effects::EffectReason::BindingRead {
                            return false;
                        }
                        facts
                            .references
                            .iter()
                            .find(|r| r.span == site.span)
                            .and_then(|r| r.binding)
                            .and_then(|id| facts.bindings.iter().find(|b| b.id == id))
                            .is_some_and(|binding| {
                                if binding.import {
                                    return binding_may_throw(graph, file, facts, site.span);
                                }
                                !binding.function
                                    && !binding.function_scoped
                                    && binding.declarations.iter().any(|declaration| {
                                        !effects::contains(deferred, *declaration)
                                            || declaration.start >= site.span.start
                                    })
                            })
                    })
            })
        });
    for (file, analyzed) in result.files {
        let facts = graph.file_facts(file).unwrap();
        let segments = graph.file_segments(file).unwrap();
        // Prepared CSS ESM stubs perform DOM writes and throw without a document.
        // Caller DOM declarations must never infer those operations away.
        if structural_override
            || facts.commonjs
            || graph
                .file_path(file)
                .is_some_and(crate::resource::is_stylesheet)
        {
            continue;
        }
        for root in analyzed.roots {
            let segment = segments
                .iter()
                .position(|segment| effects::contains(segment.span, root.span))
                .ok_or_eyre("Ecco root does not belong to a source segment")?;
            let key = SegmentKey::new(file, segment as u32);
            ensure!(
                effects
                    .insert(key, result.evaluation.effect(root.effects))
                    .is_none(),
                "multiple Ecco roots for a source segment"
            );
        }
    }
    Ok(effects)
}

pub(crate) fn summarize(
    graph: &SourceGraph,
    file: u32,
    facts: &ModuleFacts,
    span: oxc_span::Span,
    demanded: bool,
    policy: &EffectPolicy,
    effectful: Option<bool>,
) -> effects::StatementEffects {
    let mut summary = effects::summarize_with_policy(facts, span, demanded, policy);
    if let Some(false) = effectful {
        // Binding reads remain native TDZ/error barriers. Ecco intentionally
        // does not model throws as effects; emission must preserve these.
        summary.sites.retain(|site| {
            (matches!(
                site.reason,
                ast_segmenter::effects::EffectReason::Invocation
                    | ast_segmenter::effects::EffectReason::Construction
            ) && facts
                .invocations
                .iter()
                .any(|call| call.span == site.span && call.pure.is_some()))
                || site.reason == ast_segmenter::effects::EffectReason::BindingRead
                    && binding_may_throw(graph, file, facts, site.span)
        });
    }
    summary
}

fn binding_may_throw(
    graph: &SourceGraph,
    file: u32,
    facts: &ModuleFacts,
    span: oxc_span::Span,
) -> bool {
    let Some(binding) = facts
        .references
        .iter()
        .find(|reference| reference.span == span)
        .and_then(|reference| reference.binding)
        .and_then(|id| facts.bindings.iter().find(|binding| binding.id == id))
    else {
        return true;
    };
    if binding.import {
        let Some(import) = facts
            .imports
            .iter()
            .find(|import| import.binding == binding.id)
        else {
            return true;
        };
        let Some(DependencyTarget::File(target)) = graph.dependency_target(file, import.dependency)
        else {
            return true;
        };
        let Some(target) = graph.file_id(target) else {
            return true;
        };
        let ast_segmenter::owned::ImportedName::Name(name) = &import.imported else {
            return false;
        };
        return !graph
            .resolve_export(target, name, Namespace::Value)
            .is_ok_and(|linked| match linked.identity {
                ExportIdentity::Binding(file, id) => graph.file_facts(file).is_some_and(|facts| {
                    facts
                        .bindings
                        .iter()
                        .any(|binding| binding.id == id && binding.function)
                }),
                ExportIdentity::Namespace(_) => true,
                _ => false,
            });
    }
    if binding.function_scoped || binding.function {
        return false;
    }
    let Some(declaration) = binding.declarations.first() else {
        return true;
    };
    declaration.start >= span.start
        || graph.file_segments(file).is_some_and(|segments| {
            segments.iter().any(|segment| {
                effects::contains(segment.span, *declaration)
                    && effects::contains(segment.span, span)
            })
        })
}

fn resolve(
    graph: &SourceGraph,
    file: u32,
    reference: &ecco::ExternalReference,
) -> eyre::Result<ecco::Resolution<u32>> {
    let facts = graph.file_facts(file).ok_or_eyre("missing effect source")?;
    let parsed = facts.ecco.as_ref().ok_or_eyre("missing Ecco precursors")?;
    if ecco::declaration_matches(parsed.declarations(), &reference.binding) {
        return Ok(ecco::Resolution::Declaration(reference.binding.clone()));
    }
    let ecco::Binding::Module { module, path } = &reference.binding else {
        return Ok(ecco::Resolution::Unknown);
    };
    let dependency = facts
        .dependencies
        .iter()
        .find(|dependency| {
            dependency.specifier == *module
                && !dependency.type_only
                && matches!(
                    dependency.kind,
                    DependencyKind::Import | DependencyKind::ReExport
                )
        })
        .ok_or_eyre("effect import has no structural dependency")?;
    match graph.dependency_target(file, dependency.id) {
        Some(DependencyTarget::External(_)) => Ok(ecco::Resolution::Unknown),
        Some(DependencyTarget::File(target)) => {
            let file = graph
                .file_id(target)
                .ok_or_eyre("missing effect import target")?;
            exported(graph, file, path)
        }
        _ => eyre::bail!("unresolved effect import {module:?}"),
    }
}

fn exported(
    graph: &SourceGraph,
    file: u32,
    path: &[String],
) -> eyre::Result<ecco::Resolution<u32>> {
    let Some((name, rest)) = path.split_first() else {
        if graph.file_facts(file).is_some_and(|facts| facts.commonjs) {
            return Ok(ecco::Resolution::Unknown);
        }
        return Ok(ecco::Resolution::Namespace(file));
    };
    let Some(linked) = graph.resolve_optional_export(file, name, Namespace::Value)? else {
        return Ok(ecco::Resolution::Unknown);
    };
    match linked.identity {
        ExportIdentity::Namespace(file) => exported(graph, file, rest),
        _ if !rest.is_empty() => Ok(ecco::Resolution::Unknown),
        ExportIdentity::Binding(file, binding) => Ok(ecco::Resolution::Symbol(ecco::Symbol {
            file,
            local: ecco::SymbolId(binding.0),
        })),
        ExportIdentity::Expression(segment) => {
            let facts = graph.file_facts(segment.file_id).unwrap();
            let parsed = facts
                .ecco
                .as_ref()
                .ok_or_eyre("missing default precursors")?;
            let span =
                graph.file_segments(segment.file_id).unwrap()[segment.segment_idx as usize].span;
            let root = parsed
                .roots()
                .iter()
                .position(|root| effects::contains(span, root.span))
                .ok_or_eyre("default export has no Ecco root")?;
            Ok(ecco::Resolution::Root {
                file: segment.file_id,
                root: ecco::RootId(root),
            })
        }
        ExportIdentity::CommonJs(..)
        | ExportIdentity::External(..)
        | ExportIdentity::Native(..) => Ok(ecco::Resolution::Unknown),
    }
}
