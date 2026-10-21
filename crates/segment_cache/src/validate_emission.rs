use std::collections::{HashMap, HashSet};

use ast_segmenter::{
    emission::*,
    owned::{BindingId, DependencyKind, ExportValue, ModuleFacts},
};
use eyre::{ensure, OptionExt};
use oxc_span::Span;

use crate::validate::{contains, span};
use crate::CachedModule;

fn strict(values: impl IntoIterator<Item = u32>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|p| p >= value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

fn within(source: &str, outer: Span, inner: Span) -> eyre::Result<()> {
    span(source, inner)?;
    ensure!(
        inner.start < inner.end && contains(outer, inner),
        "invalid emission containment"
    );
    Ok(())
}

pub(crate) fn validate(module: &CachedModule, source: &str) -> eyre::Result<()> {
    let facts = &module.facts;
    let emission = &facts.emission;
    ensure!(
        emission.segments.len() == module.segments.len(),
        "emission segment count mismatch"
    );
    ensure!(
        emission
            .dynamic_namespaces
            .iter()
            .map(|n| n.dependency)
            .eq(facts
                .dependencies
                .iter()
                .filter(|d| d.kind == DependencyKind::DynamicImport)
                .map(|d| d.id)),
        "invalid dynamic namespace dependency links"
    );
    for observation in &emission.dynamic_namespaces {
        if let Some(properties) = &observation.properties {
            ensure!(
                properties.windows(2).all(|w| w[0] < w[1]),
                "invalid dynamic namespace properties"
            );
        }
    }
    ensure!(
        emission.nested_bindings
            == facts
                .bindings
                .iter()
                .filter(|b| !b.top_level)
                .map(|b| b.id)
                .collect::<Vec<_>>()
            && emission.free_references
                == facts
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.binding.is_none())
                    .map(|(i, _)| i as u32)
                    .collect::<Vec<_>>(),
        "invalid emission capture links"
    );
    for (&range, recipe) in module.segments.iter().zip(&emission.segments) {
        let pieces = recipe.source.as_slice();
        ensure!(
            !pieces.is_empty() && pieces.len() <= 4,
            "invalid source pieces"
        );
        for &piece in pieces {
            span(source, piece)?;
        }
        ensure!(
            recipe.source.contains(range)
                && pieces.windows(2).all(|w| w[0].end <= w[1].start)
                && (pieces == [range]
                    || (pieces.len() == 2 && pieces[1] == range && pieces[0].end <= range.start)),
            "invalid segment source assembly"
        );
        placement(facts, recipe, range)?;
        namespaces(facts, recipe, source, range)?;
        operations(facts, recipe, source, range)?;
        if facts.commonjs {
            ensure!(
                !recipe.connector_only
                    && recipe.placement == PlacementFacts::default()
                    && recipe.deferred == DeferredForm::Unsupported
                    && recipe.names == NameRestrictions::default()
                    && recipe.namespaces.is_empty()
                    && recipe.operations.iter().all(|o| matches!(
                        o.kind,
                        OperationKind::Dependency { .. } | OperationKind::SourceHint { .. }
                    )),
                "nonopaque CommonJS emission recipe"
            );
        }
    }
    Ok(())
}

fn placement(facts: &ModuleFacts, recipe: &SegmentRecipe, range: Span) -> eyre::Result<()> {
    let placement = &recipe.placement;
    if facts.commonjs {
        return Ok(());
    }
    let definitions: Vec<_> = facts
        .bindings
        .iter()
        .filter(|b| b.top_level && !b.import && b.declarations.iter().any(|&s| contains(range, s)))
        .collect();
    ensure!(
        placement.declarations == definitions.iter().map(|b| b.id).collect::<Vec<_>>()
            && placement.forward_references
                == definitions
                    .iter()
                    .filter(|b| {
                        facts
                            .references
                            .iter()
                            .any(|r| r.binding == Some(b.id) && r.span.start < range.start)
                    })
                    .map(|b| b.id)
                    .collect::<Vec<_>>(),
        "invalid emission declaration/forward links"
    );
    ensure!(
        strict(placement.hoisted.iter().map(|b| b.0))
            && placement
                .hoisted
                .iter()
                .all(|id| definitions.iter().any(|b| b.id == *id && b.function))
            && placement.instantiable
                == (!definitions.is_empty() && placement.declarations == placement.hoisted),
        "invalid hoisted availability"
    );
    ensure!(
        placement.default_expression
            == facts
                .exports
                .iter()
                .any(|e| matches!(e.value, ExportValue::Expression(s) if contains(range, s))),
        "invalid default-expression placement"
    );
    ensure!(
        strict(placement.eager_reads.iter().copied())
            && placement.eager_reads.iter().all(|&i| facts
                .references
                .get(i as usize)
                .is_some_and(|r| r.read && contains(range, r.span)))
            && placement.writes
                == facts
                    .references
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.write && contains(range, r.span))
                    .map(|(i, _)| i as u32)
                    .collect::<Vec<_>>(),
        "invalid eager-read/write links"
    );
    ensure!(
        strict(recipe.names.named.iter().map(|b| b.0))
            && recipe.names.named.iter().all(|id| facts
                .bindings
                .iter()
                .any(|b| b.id == *id && b.declarations.iter().any(|&s| contains(range, s)))),
        "invalid name-sensitive binding links"
    );
    Ok(())
}

fn namespaces(
    facts: &ModuleFacts,
    recipe: &SegmentRecipe,
    source: &str,
    range: Span,
) -> eyre::Result<()> {
    let expected: Vec<_> = facts
        .references
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            !facts.commonjs
                && contains(range, r.span)
                && facts.imports.iter().any(|i| Some(i.binding) == r.binding)
        })
        .map(|(i, _)| i as u32)
        .collect();
    ensure!(
        recipe
            .namespaces
            .iter()
            .map(|u| u.reference)
            .collect::<Vec<_>>()
            == expected,
        "invalid namespace reference links"
    );
    for usage in &recipe.namespaces {
        if let NamespaceAccess::DirectRead { member, property } = &usage.access {
            within(source, range, *member)?;
            let reference = &facts.references[usage.reference as usize];
            ensure!(
                member.start == reference.span.start
                    && reference.span.end < member.end
                    && !property.is_empty(),
                "invalid namespace member range"
            );
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Key {
    Binding(BindingId, u32),
    Reference(u32),
    Namespace(u32),
    Dependency(u32),
    Export(u32),
    Hint(u32),
    Invocation(u32),
}

fn operations(
    facts: &ModuleFacts,
    recipe: &SegmentRecipe,
    source: &str,
    range: Span,
) -> eyre::Result<()> {
    let bindings: HashMap<_, _> = facts.bindings.iter().map(|b| (b.id, b)).collect();
    let mut keys = HashSet::new();
    let mut ranges = Vec::with_capacity(recipe.operations.len());
    let mut connector = false;
    for operation in &recipe.operations {
        let key = match &operation.kind {
            OperationKind::Binding {
                binding,
                declaration,
                shorthand,
            } => {
                let binding = bindings
                    .get(binding)
                    .ok_or_eyre("dangling emission binding")?;
                binding
                    .declarations
                    .get(*declaration as usize)
                    .ok_or_eyre("dangling emission declaration")?;
                ensure!(!facts.commonjs, "lexical CJS rewrite");
                shorthand_check(&binding.name, *shorthand)?;
                Key::Binding(binding.id, *declaration)
            }
            OperationKind::Reference {
                reference,
                shorthand,
            } => {
                let reference_record = facts
                    .references
                    .get(*reference as usize)
                    .ok_or_eyre("dangling emission reference")?;
                ensure!(!facts.commonjs, "lexical CJS rewrite");
                shorthand_check(&reference_record.name, *shorthand)?;
                Key::Reference(*reference)
            }
            OperationKind::Namespace { usage } => {
                let usage_record = recipe
                    .namespaces
                    .get(*usage as usize)
                    .ok_or_eyre("dangling emission namespace")?;
                ensure!(
                    matches!(usage_record.access, NamespaceAccess::DirectRead { .. }),
                    "native namespace has direct-read operation"
                );
                Key::Namespace(*usage)
            }
            OperationKind::Dependency { dependency } => {
                let dependency = facts
                    .dependencies
                    .iter()
                    .find(|d| d.id == *dependency)
                    .ok_or_eyre("dangling emission dependency")?;
                ensure!(
                    matches!(
                        dependency.kind,
                        DependencyKind::Import
                            | DependencyKind::DynamicImport
                            | DependencyKind::Require
                    ),
                    "invalid emission dependency kind"
                );
                if dependency.kind == DependencyKind::Import {
                    ensure!(dependency.span == range, "nonstatement import operation");
                    connector = true;
                }
                Key::Dependency(dependency.id.0)
            }
            OperationKind::Export {
                span: wrapper,
                form,
            } => {
                ensure!(
                    !facts.commonjs
                        && (*wrapper == range
                            || (recipe.source.as_slice().len() > 1
                                && *wrapper == recipe.source.as_slice()[0]
                                && matches!(form, ExportForm::Declaration { .. }))),
                    "invalid export wrapper range"
                );
                ensure!(
                    source
                        .get(wrapper.start as usize..wrapper.end as usize)
                        .is_some_and(|s| s.starts_with("export")),
                    "invalid export wrapper text"
                );
                match form {
                    ExportForm::Remove => {
                        ensure!(
                            recipe.placement.declarations.is_empty()
                                && !facts.exports.iter().any(|e| e.span == *wrapper
                                    && matches!(e.value, ExportValue::Expression(_))),
                            "removed default value"
                        );
                        connector = true;
                    }
                    ExportForm::Declaration { declaration } => {
                        within(source, *wrapper, *declaration)?;
                        ensure!(
                            wrapper.start < declaration.start && declaration.end == wrapper.end,
                            "invalid export declaration wrapper"
                        );
                        ensure!(
                            !facts.exports.iter().any(|e| contains(*wrapper, e.span)
                                && !matches!(e.value, ExportValue::Binding(_))),
                            "invalid export declaration facts"
                        );
                    }
                    ExportForm::Default { value, .. } => {
                        ensure!(
                            facts.exports.iter().any(|e| e.span == *wrapper
                                && e.name.as_deref() == Some("default")
                                && e.value == ExportValue::Expression(value.span)),
                            "invalid default value link"
                        );
                    }
                }
                Key::Export(wrapper.start)
            }
            OperationKind::SourceHint { span: hint } => {
                span(source, *hint)?;
                ensure!(
                    source_hint(&source[hint.start as usize..hint.end as usize]),
                    "invalid source hint text"
                );
                Key::Hint(hint.start)
            }
            OperationKind::Invocation { invocation, .. } => {
                ensure!(
                    !facts.commonjs && facts.invocations.get(*invocation as usize).is_some(),
                    "dangling emission invocation"
                );
                Key::Invocation(*invocation)
            }
        };
        ensure!(keys.insert(key), "duplicate emission operation");
        let operation_range = operation.kind.span(facts, recipe);
        span(source, operation_range)?;
        ensure!(
            operation_range.start < operation_range.end && recipe.source.contains(operation_range),
            "invalid emission containment"
        );
        ranges.push(operation_range);
    }
    if !facts.commonjs {
        connector |= source
            .get(range.start as usize..range.end as usize)
            .is_some_and(|s| s.trim() == ";");
        ensure!(
            recipe.connector_only == connector,
            "invalid connector classification"
        );
    }
    let mut stack: Vec<usize> = Vec::new();
    let mut previous = None;
    for (i, (&operation_range, operation)) in ranges.iter().zip(&recipe.operations).enumerate() {
        let key = (
            operation_range.start,
            std::cmp::Reverse(operation_range.end),
        );
        ensure!(
            previous.is_none_or(|p| p <= key),
            "unordered emission operations"
        );
        previous = Some(key);
        while stack
            .last()
            .is_some_and(|&p| !contains(ranges[p], operation_range))
        {
            let p = stack.pop().expect("nonempty stack");
            ensure!(
                ranges[p].end <= operation_range.start,
                "crossing emission operations"
            );
        }
        ensure!(
            operation.parent == stack.last().map(|&p| p as u32),
            "invalid emission parent composition"
        );
        stack.push(i);
    }
    for (owner, operation) in recipe.operations.iter().enumerate() {
        match &operation.kind {
            OperationKind::Export {
                span: outer,
                form: ExportForm::Default { value, .. },
            } => source_slot(source, *outer, value, owner, &ranges, &recipe.operations)?,
            OperationKind::Invocation {
                invocation,
                lowering,
            } => pure(facts, *invocation, lowering, owner, recipe, source, &ranges)?,
            _ => {}
        }
    }
    match &recipe.deferred {
        DeferredForm::Unsupported => {}
        DeferredForm::Function => ensure!(
            recipe.placement.instantiable && !recipe.placement.default_expression,
            "invalid deferred function"
        ),
        DeferredForm::Effect => ensure!(
            recipe.placement.declarations.is_empty() && !recipe.placement.default_expression,
            "invalid deferred effect"
        ),
        DeferredForm::Variable {
            binding,
            initializer,
        } => {
            let binding = bindings
                .get(binding)
                .ok_or_eyre("dangling deferred binding")?;
            ensure!(
                !facts.commonjs
                    && !binding.import
                    && !binding.function
                    && recipe.placement.declarations == [binding.id]
                    && !recipe.placement.default_expression,
                "invalid deferred variable"
            );
            if let Some(slot) = initializer {
                within(source, range, slot.span)?;
                ensure!(
                    binding
                        .declarations
                        .iter()
                        .any(|s| contains(range, *s) && s.end < slot.span.start),
                    "invalid deferred initializer range"
                );
                let expected: Vec<_> = ranges
                    .iter()
                    .enumerate()
                    .filter(|&(i, &range)| {
                        contains(slot.span, range)
                            && !recipe.operations[i]
                                .parent
                                .is_some_and(|p| contains(slot.span, ranges[p as usize]))
                    })
                    .map(|(i, _)| i as u32)
                    .collect();
                ensure!(
                    slot.operations == expected,
                    "invalid deferred source-slot composition"
                );
            }
        }
    }
    for (binding, info) in &bindings {
        for (i, &declaration) in info.declarations.iter().enumerate() {
            if !facts.commonjs && contains(range, declaration) {
                ensure!(
                    keys.contains(&Key::Binding(*binding, i as u32)),
                    "missing emission declaration"
                );
            }
        }
    }
    for (i, reference) in facts.references.iter().enumerate() {
        if !facts.commonjs && contains(range, reference.span) {
            ensure!(
                keys.contains(&Key::Reference(i as u32)),
                "missing emission reference"
            );
        }
    }
    for (i, usage) in recipe.namespaces.iter().enumerate() {
        if matches!(usage.access, NamespaceAccess::DirectRead { .. }) {
            ensure!(
                keys.contains(&Key::Namespace(i as u32)),
                "missing namespace operation"
            );
        }
    }
    for dependency in &facts.dependencies {
        if contains(range, dependency.span)
            && matches!(
                dependency.kind,
                DependencyKind::Import | DependencyKind::DynamicImport | DependencyKind::Require
            )
        {
            ensure!(
                keys.contains(&Key::Dependency(dependency.id.0)),
                "missing emission dependency"
            );
        }
    }
    for (i, invocation) in facts.invocations.iter().enumerate() {
        if contains(range, invocation.span) {
            ensure!(
                keys.contains(&Key::Invocation(i as u32)),
                "missing emission invocation"
            );
        }
    }
    for export in &facts.exports {
        if contains(range, export.span) {
            ensure!(
                keys.contains(&Key::Export(recipe.source.as_slice()[0].start)),
                "missing export wrapper"
            );
        }
    }
    Ok(())
}

fn shorthand_check(name: &str, shorthand: Shorthand) -> eyre::Result<()> {
    if shorthand == Shorthand::Proto {
        ensure!(name == "__proto__", "invalid literal proto shorthand");
    }
    Ok(())
}

fn source_hint(text: &str) -> bool {
    text.strip_prefix("//")
        .or_else(|| text.strip_prefix("/*").and_then(|t| t.strip_suffix("*/")))
        .and_then(|b| b.strip_prefix('#').or_else(|| b.strip_prefix('@')))
        .is_some_and(|b| {
            b.trim_start().starts_with("sourceMappingURL=")
                || b.trim_start().starts_with("sourceURL=")
        })
}

fn source_slot(
    source: &str,
    outer: Span,
    slot: &SourceSlot,
    owner: usize,
    ranges: &[Span],
    operations: &[Operation],
) -> eyre::Result<()> {
    within(source, outer, slot.span)?;
    ensure!(
        slot.span != ranges[owner] && slot.operations.iter().all(|&i| i as usize > owner),
        "recursive source-slot composition"
    );
    let expected: Vec<_> = ranges
        .iter()
        .enumerate()
        .filter(|&(i, &range)| {
            contains(slot.span, range)
                && !operations[i]
                    .parent
                    .is_some_and(|p| contains(slot.span, ranges[p as usize]))
        })
        .map(|(i, _)| i as u32)
        .collect();
    ensure!(
        slot.operations == expected,
        "invalid source-slot composition"
    );
    Ok(())
}

fn pure(
    facts: &ModuleFacts,
    invocation: u32,
    lowering: &PureLowering,
    owner: usize,
    recipe: &SegmentRecipe,
    source: &str,
    ranges: &[Span],
) -> eyre::Result<()> {
    let call = &facts.invocations[invocation as usize];
    ensure!(
        lowering.callee.span == call.callee && lowering.arguments.len() == call.arguments.len(),
        "invalid PURE input links"
    );
    source_slot(
        source,
        call.span,
        &lowering.callee,
        owner,
        ranges,
        &recipe.operations,
    )?;
    for (slot, argument) in lowering.arguments.iter().zip(&call.arguments) {
        ensure!(slot.span == argument.span, "invalid PURE argument link");
        source_slot(source, call.span, slot, owner, ranges, &recipe.operations)?;
    }
    if let Some(start) = lowering.guard_start {
        ensure!(
            call.callee.start < start
                && start <= call.callee.end
                && source.is_char_boundary(start as usize),
            "invalid PURE guard boundary"
        );
    }
    ensure!(
        !lowering.crosses_await_guard || lowering.guard_start.is_some(),
        "await veto without PURE guard"
    );
    if call.deferred.is_none() {
        ensure!(
            lowering.crosses_await_guard
                == lowering.guard_start.is_some_and(|start| facts
                    .top_level_await
                    .iter()
                    .any(|s| start <= s.start && s.end <= call.span.end)),
            "invalid module await guard veto"
        );
    }
    if let Some(chain) = &lowering.chain {
        source_slot(
            source,
            call.callee,
            &chain.base,
            owner,
            ranges,
            &recipe.operations,
        )?;
        ensure!(
            chain.base.span.start == call.callee.start,
            "invalid PURE chain base"
        );
        let optional = |step: &ChainStep| {
            matches!(
                step,
                ChainStep::Static { optional: true, .. }
                    | ChainStep::Computed { optional: true, .. }
            )
        };
        if chain.steps.iter().any(optional) {
            ensure!(
                lowering
                    .guard_start
                    .is_some_and(|start| chain.base.span.end <= start && start < call.callee.end),
                "invalid member guard boundary"
            );
            if chain.steps.first().is_some_and(optional) {
                ensure!(
                    lowering.guard_start == Some(chain.base.span.end),
                    "invalid first member guard"
                );
            }
        } else {
            ensure!(
                lowering.guard_start == call.optional_call.then_some(call.callee.end),
                "invalid unguarded chain boundary"
            );
        }
        let mut end = chain.base.span.end;
        for step in &chain.steps {
            let slot = match step {
                ChainStep::Static { property, .. } => {
                    ensure!(!property.is_empty(), "empty PURE member property");
                    None
                }
                ChainStep::Computed { property, .. } => Some(property),
                ChainStep::Call { suffix } => Some(suffix),
            };
            if let Some(slot) = slot {
                source_slot(source, call.callee, slot, owner, ranges, &recipe.operations)?;
                ensure!(end <= slot.span.start, "unordered PURE chain slots");
                end = slot.span.end;
            }
        }
        if chain.steps.is_empty() {
            ensure!(
                chain.base.span == call.callee,
                "incomplete opaque PURE callee"
            );
        }
    }
    Ok(())
}
