//! Scheduling inputs only. These relationships must never participate in demand traversal.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use ast_segmenter::{effects::EffectReason, owned::BindingId, views::ImportedName};
use eyre::OptionExt;
use source_graph::{
    DependencyTarget, ExportIdentity, GraphNode, Namespace, SegmentKey, SourceGraph,
};

use crate::{RetentionEdge, RetentionEdgeKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub constraints: Vec<ExecutionConstraint>,
    /// Direct canonical relationships, not transitive symbol sets. Cross-source
    /// reads preserve native source phases until an entrance-aware scheduler proves
    /// placement; they are deliberately NOT declaration-before-read edges.
    pub accesses: Vec<LinkedAccess>,
    /// Fail-closed source-phase fences, located at their evidence segment.
    /// Until discharged by a scheduling proof, preserve the owning source's native
    /// phase, including its other mutable reads/initializers, not just this site.
    /// These are not permission to run a deferred context, nor purity conclusions.
    pub barriers: Vec<ExecutionBarrier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConstraintKind {
    Initialization,
    PreInitialization,
    ReadWrite,
    /// Preserve both sides of a potentially abrupt source evaluation phase.
    AbruptCompletion,
    /// The only breakable kind; mandatory constraints never inherit SCC breaking.
    EffectOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExecutionConstraint {
    pub before: SegmentKey,
    pub after: SegmentKey,
    pub kind: ConstraintKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkedAccess {
    pub segment: SegmentKey,
    pub reference: u32,
    /// Real declaration recipe, bypassing reexport/import connectors.
    pub declaration: SegmentKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BarrierReason {
    UnknownInvocation,
    /// Eager syntax may execute user code without a Call/New occurrence.
    UnknownExecution,
    /// Eager syntax may leave later source initializations unexecuted.
    AbruptCompletion,
    DeferredAccess,
    CrossSourceAccess,
    Suspension,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExecutionBarrier {
    pub segment: SegmentKey,
    pub reason: BarrierReason,
}

#[derive(Clone, Copy)]
struct Event {
    segment: SegmentKey,
    offset: u32,
    read: bool,
    write: bool,
    initialization: bool,
}

pub(crate) fn plan(
    graph: &SourceGraph,
    retained: &BTreeSet<SegmentKey>,
    edges: &[RetentionEdge],
    lazy: Option<&crate::lazy_modules::Certification>,
    effects: &BTreeMap<SegmentKey, crate::effects::StatementEffects>,
) -> eyre::Result<ExecutionPlan> {
    let mut constraints = BTreeSet::new();
    let mut accesses = BTreeSet::new();
    let mut barriers = BTreeSet::new();
    let mut events: HashMap<(u32, BindingId), Vec<Event>> = HashMap::new();
    let mut declarations = HashMap::new();
    let mut imported_declarations = HashMap::new();
    let mut local_phases: BTreeMap<u32, Vec<SegmentKey>> = BTreeMap::new();
    let mut abrupt_phases = BTreeSet::new();
    for &segment in retained {
        let facts = graph
            .file_facts(segment.file_id)
            .ok_or_eyre("missing execution facts")?;
        let Some(recipe) = facts
            .emission()
            .segments()
            .get(segment.segment_idx as usize)
        else {
            // Analysis-only graphs cannot grant relocation permission.
            barriers.insert(ExecutionBarrier {
                segment,
                reason: BarrierReason::Opaque,
            });
            continue;
        };
        if !recipe.connector_only() && !recipe.placement().instantiable() {
            local_phases
                .entry(segment.file_id)
                .or_default()
                .push(segment);
        }
        for binding in recipe.placement().declarations() {
            if lazy.is_some_and(|lazy| lazy.consumes_binding(segment.file_id, binding)) {
                continue;
            }
            declarations
                .entry((segment.file_id, binding))
                .or_insert(segment);
            if !recipe.placement().hoisted().contains(&binding) {
                events
                    .entry((segment.file_id, binding))
                    .or_default()
                    .push(Event {
                        segment,
                        offset: 0,
                        read: false,
                        write: true,
                        initialization: true,
                    });
            }
        }
    }
    for &segment in retained {
        let file = segment.file_id;
        let facts = graph
            .file_facts(file)
            .ok_or_eyre("missing execution facts")?;
        let ranges = graph
            .file_segments(file)
            .ok_or_eyre("missing execution ranges")?;
        let range = ranges[segment.segment_idx as usize].span;
        let timing = &facts.emission().execution();
        let mut pin = |reason| {
            barriers.insert(ExecutionBarrier { segment, reason });
        };
        if facts.commonjs() || facts.emission().segments().is_empty() {
            pin(BarrierReason::Opaque);
            continue;
        }
        if !facts.top_level_await().is_empty() {
            pin(BarrierReason::Suspension);
        }
        // Use original syntax evidence, never annotation/solver-derived purity.
        // Calls are not the only way to execute unknown code: property access,
        // coercion, iteration and unsupported expressions (including tags) can too.
        let effect_start = facts
            .effects()
            .partition_point(|e| e.span().start < range.start);
        for site in facts
            .effects()
            .slice(effect_start..)
            .iter()
            .take_while(|e| e.span().start < range.end)
            .filter(|e| crate::effects::contains(range, e.span()))
        {
            if lazy.is_some_and(|lazy| {
                matches!(
                    site.reason(),
                    EffectReason::Invocation | EffectReason::Construction
                ) && lazy.allocation(segment, site.span())
                    && !effects.get(&segment).is_some_and(|summary| {
                        summary
                            .known_effects
                            .iter()
                            .any(|effect| effect.invocation == site.span())
                    })
                    || site.reason() == EffectReason::BindingRead
                        && lazy.consumed_read(graph, segment, site.span())
            }) {
                continue;
            }
            if !timing.effect_is_module_evaluation(site)? {
                continue;
            }
            pin(BarrierReason::AbruptCompletion);
            abrupt_phases.insert(segment);
            if !matches!(
                site.reason(),
                EffectReason::Invocation | EffectReason::Construction | EffectReason::BindingRead
            ) {
                pin(BarrierReason::UnknownExecution);
            }
        }
        let call_start = facts
            .invocations()
            .partition_point(|c| c.span().start < range.start);
        for (relative, call) in facts
            .invocations()
            .slice(call_start..)
            .iter()
            .enumerate()
            .take_while(|(_, c)| c.span().start < range.end)
            .filter(|(_, c)| crate::effects::contains(range, c.span()))
        {
            if lazy.is_some_and(|lazy| {
                lazy.allocation(segment, call.span())
                    && !effects.get(&segment).is_some_and(|summary| {
                        summary
                            .known_effects
                            .iter()
                            .any(|effect| effect.invocation == call.span())
                    })
            }) {
                continue;
            }
            let index = call_start + relative;
            if !timing
                .contexts()
                .iter()
                .any(|c| c.immediate() == Some(index as u32))
            {
                pin(BarrierReason::UnknownInvocation);
            }
            // Even syntactically immediate calls inside deferred contexts do not
            // authorize moving the surrounding callable's captured accesses.
            if !timing.is_module_evaluation(timing.invocations().get(index).unwrap()) {
                pin(BarrierReason::UnknownInvocation);
            }
        }
        let reference_start = facts
            .references()
            .partition_point(|r| r.span().start < range.start);
        for (relative, reference) in facts
            .references()
            .slice(reference_start..)
            .iter()
            .enumerate()
            .take_while(|(_, r)| r.span().start < range.end)
            .filter(|(_, r)| !r.type_only() && crate::effects::contains(range, r.span()))
        {
            let index = reference_start + relative;
            if lazy.is_some_and(|lazy| lazy.consumed_constructor(segment, index as u32)) {
                continue;
            }
            if reference.binding().is_some_and(|binding| {
                !graph.runtime_import(file, binding)
                    || lazy.is_some_and(|lazy| lazy.consumes_binding(file, binding))
            }) {
                continue;
            }
            let Some(binding) = reference.binding().and_then(|id| {
                facts
                    .bindings()
                    .iter()
                    .find(|b| b.id() == id && b.top_level())
            }) else {
                continue;
            };
            let eager = timing.is_module_evaluation(timing.references().get(index).unwrap());
            if !eager {
                pin(BarrierReason::DeferredAccess);
            }
            let declaration = if binding.import() {
                if let Some(&declaration) = imported_declarations.get(&(file, binding.id())) {
                    declaration
                } else {
                    let import = facts
                        .imports()
                        .iter()
                        .find(|i| i.binding() == binding.id())
                        .ok_or_eyre("missing execution import")?;
                    let declaration = match (
                        graph.dependency_target(file, import.dependency()),
                        &import.imported(),
                    ) {
                        (Some(DependencyTarget::File(path)), ImportedName::Name(name)) => {
                            let target =
                                graph.file_id(path).ok_or_eyre("missing execution target")?;
                            let linked = graph.resolve_export(target, &name, Namespace::Value)?;
                            match linked.identity {
                                ExportIdentity::Binding(owner, id) => {
                                    declarations.get(&(owner, id)).copied()
                                }
                                ExportIdentity::Expression(key) => Some(key),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    imported_declarations.insert((file, binding.id()), declaration);
                    declaration
                }
            } else {
                declarations.get(&(file, binding.id())).copied()
            };
            if let Some(declaration) = declaration.filter(|key| retained.contains(key)) {
                accesses.insert(LinkedAccess {
                    segment,
                    reference: index as u32,
                    declaration,
                });
                if declaration.file_id != file {
                    pin(BarrierReason::CrossSourceAccess);
                }
            } else {
                pin(BarrierReason::Opaque);
            }
            if eager && !binding.import() {
                events.entry((file, binding.id())).or_default().push(Event {
                    segment,
                    offset: reference.span().start,
                    read: reference.read(),
                    write: reference.write(),
                    initialization: false,
                });
            }
        }
    }
    for mut events in events.into_values() {
        events.sort_by_key(|event| (event.segment.segment_idx, event.offset));
        let mut writer: Option<Event> = None;
        let mut readers = BTreeSet::new();
        for event in events {
            if event.read {
                if let Some(previous) = writer {
                    add(
                        &mut constraints,
                        previous.segment,
                        event.segment,
                        if previous.initialization {
                            ConstraintKind::Initialization
                        } else {
                            ConstraintKind::ReadWrite
                        },
                    );
                }
                readers.insert(event.segment);
            }
            if event.write {
                if let Some(previous) = writer {
                    add(
                        &mut constraints,
                        previous.segment,
                        event.segment,
                        ConstraintKind::ReadWrite,
                    );
                }
                for reader in std::mem::take(&mut readers) {
                    add(
                        &mut constraints,
                        reader,
                        event.segment,
                        if writer.is_none() && event.initialization {
                            ConstraintKind::PreInitialization
                        } else {
                            ConstraintKind::ReadWrite
                        },
                    );
                }
                writer = Some(event);
            }
        }
    }
    // A throw leaves *all* later bindings uninitialized, not only the binding
    // read at the throwing site. Earlier initializations must also stay earlier.
    // Fence only local evaluation phases; hoisted functions/connectors and other
    // sources do not acquire an arbitrary total order. Chain adjacent fences and
    // their intervening phases, keeping peer initializers unordered and size linear.
    for phases in local_phases.into_values() {
        let mut previous_fence = None;
        let mut pending = Vec::new();
        for phase in phases {
            if let Some(previous) = previous_fence {
                add(
                    &mut constraints,
                    previous,
                    phase,
                    ConstraintKind::AbruptCompletion,
                );
            }
            if abrupt_phases.contains(&phase) {
                for previous in pending.drain(..) {
                    add(
                        &mut constraints,
                        previous,
                        phase,
                        ConstraintKind::AbruptCompletion,
                    );
                }
                previous_fence = Some(phase);
            } else {
                pending.push(phase);
            }
        }
    }
    for edge in edges
        .iter()
        .filter(|e| e.kind == RetentionEdgeKind::EffectOrder)
    {
        if let (GraphNode::Segment(after), GraphNode::Segment(before)) = (edge.from, edge.to) {
            if retained.contains(&before) && retained.contains(&after) {
                add(&mut constraints, before, after, ConstraintKind::EffectOrder);
            }
        }
    }
    Ok(ExecutionPlan {
        constraints: constraints.into_iter().collect(),
        accesses: accesses.into_iter().collect(),
        barriers: barriers.into_iter().collect(),
    })
}

fn add(
    constraints: &mut BTreeSet<ExecutionConstraint>,
    before: SegmentKey,
    after: SegmentKey,
    kind: ConstraintKind,
) {
    if before != after {
        constraints.insert(ExecutionConstraint {
            before,
            after,
            kind,
        });
    }
}
