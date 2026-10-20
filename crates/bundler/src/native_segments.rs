//! Physical ownership for pure declarations, independent of source body seams.
use super::*;
use crate::assembly::{Code, RequestKind};
use crate::execution_dependencies::{BarrierReason, ConstraintKind};
use crate::RetentionEdgeKind;

pub(super) struct Placement {
    pub owners: BTreeMap<SegmentKey, Owner>,
    schedules: BTreeMap<Owner, Vec<SegmentKey>>,
    phase_dependencies: BTreeMap<Owner, BTreeSet<Owner>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owner {
    Evaluation(u32),
    Bindings(SegmentKey),
}

impl Owner {
    pub fn path(self, graph: &SourceGraph) -> PathBuf {
        match self {
            Self::Evaluation(file) => path(graph, file),
            Self::Bindings(key) => PathBuf::from(format!(
                "native/bindings-{}-{}.mjs",
                digest(graph.file_path(key.file_id).unwrap().to_str().unwrap()),
                key.segment_idx
            )),
        }
    }
}

impl Placement {
    pub fn schedule(&self, owner: Owner) -> &[SegmentKey] {
        self.schedules
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[expect(clippy::too_many_arguments)]
    pub fn new(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        prepared: &PreparedValues<'_>,
        seams: &BTreeMap<u32, u32>,
        phase_bound: &BTreeSet<u32>,
        wait_bound: &BTreeSet<u32>,
        surfaces: &BTreeSet<u32>,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<Self> {
        let symbols = prepared.symbols;
        let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
        if modules.keys().any(|&file| has_externals(graph, file)) {
            // Unknown external surfaces/evaluation stay on native source-module seams.
            let owners = live
                .into_iter()
                .map(|key| (key, Owner::Evaluation(seams[&key.file_id])))
                .collect();
            return Self::scheduled(graph, plan, modules, owners, &BTreeSet::new());
        }
        let effects: BTreeSet<_> = plan
            .retention
            .effects
            .iter()
            .filter(|s| s.summary.is_effectful())
            .map(|s| s.key)
            .collect();
        let keys: Vec<_> = live.iter().copied().collect();
        let indices: BTreeMap<_, _> = keys.iter().enumerate().map(|(i, &key)| (key, i)).collect();
        let mut constraints = vec![BTreeSet::new(); keys.len()];
        let mut pinned = BTreeSet::new();
        let mut declarations = BTreeSet::new();
        let mut barriers: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for barrier in &plan.retention.execution.barriers {
            barriers
                .entry(barrier.segment.file_id)
                .or_default()
                .insert(barrier.reason);
        }
        let mut fenced: BTreeSet<_> = barriers.keys().copied().collect();
        for (&file, reasons) in &barriers {
            let module = &modules[&file];
            let facts = &module.parsed.facts;
            // Local immutable reads after initialization cannot invoke user code
            // or throw. Mandatory components still carry their exact ordering.
            let safe_reads = !facts.commonjs
                && facts.top_level_await.is_empty()
                && facts.references.iter().all(|reference| !reference.write)
                && facts
                    .emission
                    .segments
                    .iter()
                    .all(|recipe| recipe.placement.forward_references.is_empty())
                && reasons.iter().all(|reason| {
                    matches!(
                        reason,
                        BarrierReason::AbruptCompletion | BarrierReason::DeferredAccess
                    )
                })
                && facts.effects.iter().all(|site| {
                    if !facts
                        .emission
                        .execution
                        .effect_is_module_evaluation(site)
                        .unwrap_or(true)
                    {
                        return true;
                    }
                    site.reason == ast_segmenter::effects::EffectReason::BindingRead
                        && facts
                            .references
                            .iter()
                            .filter(|reference| reference.span == site.span)
                            .any(|reference| {
                                reference
                                    .binding
                                    .and_then(|id| {
                                        facts.bindings.iter().find(|binding| {
                                            binding.id == id && !binding.import && binding.top_level
                                        })
                                    })
                                    .is_some_and(|binding| {
                                        binding.declarations.first().is_some_and(|span| {
                                            let index =
                                                module.parsed.segments.partition_point(|segment| {
                                                    segment.span.end <= span.start
                                                });
                                            module.parsed.segments[index].span.end
                                                <= reference.span.start
                                                || facts.emission.segments[index]
                                                    .placement
                                                    .hoisted
                                                    .contains(&binding.id)
                                        })
                                    })
                            })
                });
            if safe_reads {
                fenced.remove(&file);
            }
        }
        let mut first_abrupt = BTreeMap::new();
        for barrier in &plan.retention.execution.barriers {
            if barrier.reason == BarrierReason::AbruptCompletion {
                first_abrupt
                    .entry(barrier.segment.file_id)
                    .and_modify(|index: &mut u32| {
                        *index = (*index).min(barrier.segment.segment_idx)
                    })
                    .or_insert(barrier.segment.segment_idx);
            }
        }
        // A pure prefix may initialize in an availability job imported *after*
        // all original static requests and before the native source body. This
        // discharges the source fence without moving effects or later TDZ state.
        let prefixes: BTreeSet<_> = live
            .iter()
            .copied()
            .filter(|key| {
                let placement = &modules[&key.file_id].parsed.facts.emission.segments
                    [key.segment_idx as usize]
                    .placement;
                fenced.contains(&key.file_id)
                    && !phase_bound.contains(&key.file_id)
                    && !wait_bound.contains(&key.file_id)
                    && !cjs.contains(key.file_id)
                    && !effects.contains(key)
                    && !placement.instantiable
                    && (!placement.declarations.is_empty() || placement.default_expression)
                    && first_abrupt
                        .get(&key.file_id)
                        .is_none_or(|&index| key.segment_idx < index)
            })
            .collect();
        // Mandatory lexical relationships form indivisible components. Prefix
        // phase edges instead survive as explicit native imports; only effect
        // order can be broken, never a mandatory relationship.
        for constraint in &plan.retention.execution.constraints {
            if constraint.kind != ConstraintKind::EffectOrder {
                if constraint.kind == ConstraintKind::AbruptCompletion
                    && prefixes.contains(&constraint.before)
                    && !prefixes.contains(&constraint.after)
                {
                    continue;
                }
                if let (Some(&before), Some(&after)) = (
                    indices.get(&constraint.before),
                    indices.get(&constraint.after),
                ) {
                    constraints[before].insert(after);
                    constraints[after].insert(before);
                }
            }
        }
        for &key in &live {
            let module = &modules[&key.file_id];
            let facts = &module.parsed.facts;
            let placement = &facts.emission.segments[key.segment_idx as usize].placement;
            let definitions = placement
                .declarations
                .iter()
                .map(|id| {
                    facts
                        .bindings
                        .iter()
                        .find(|b| b.id == *id)
                        .ok_or_eyre("missing placement binding")
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            if !definitions.is_empty() || placement.default_expression {
                declarations.insert(key);
            }
            if cjs.contains(key.file_id)
                || wait_bound.contains(&key.file_id)
                || effects.contains(&key)
                || (phase_bound.contains(&key.file_id) && !placement.instantiable)
                || (fenced.contains(&key.file_id)
                    && !placement.instantiable
                    && !prefixes.contains(&key))
                || !placement.forward_references.is_empty()
            {
                pinned.insert(key);
            }
            for binding in &definitions {
                for &declaration in &binding.declarations {
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|s| s.span.end <= declaration.start);
                    let other = SegmentKey::new(key.file_id, index as u32);
                    if let Some(&other) = indices.get(&other) {
                        constraints[indices[&key]].insert(other);
                        constraints[other].insert(indices[&key]);
                    }
                }
            }
            for &index in &placement.writes {
                let reference = &facts.references[index as usize];
                let Some(binding) = reference.binding.and_then(|id| {
                    facts
                        .bindings
                        .iter()
                        .find(|b| b.id == id && b.top_level && !b.import)
                }) else {
                    continue;
                };
                let dependency =
                    declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id))?.unwrap();
                constraints[indices[&key]].insert(indices[&dependency]);
                constraints[indices[&dependency]].insert(indices[&key]);
            }
        }
        // Eager reads share their initializer's phase; deferred reads can cross
        // chunks as live imports. This also keeps snapshots ahead of later writes.
        for &key in &live {
            let module = &modules[&key.file_id];
            for &index in &module.parsed.facts.emission.segments[key.segment_idx as usize]
                .placement
                .eager_reads
            {
                let reference = &module.parsed.facts.references[index as usize];
                let Some(binding) = reference.binding.and_then(|id| {
                    module
                        .parsed
                        .facts
                        .bindings
                        .iter()
                        .find(|b| b.id == id && b.top_level)
                }) else {
                    continue;
                };
                if binding.import {
                    if let Some(linked) = symbols.imports.get(&(key.file_id, binding.id)) {
                        if let ExportIdentity::Binding(file, id) = linked.identity {
                            if modules[&file]
                                .parsed
                                .facts
                                .references
                                .iter()
                                .any(|r| r.binding == Some(id) && r.write)
                            {
                                pinned.insert(key);
                            }
                        }
                    }
                } else {
                    let dependency =
                        declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id))?
                            .unwrap();
                    constraints[indices[&key]].insert(indices[&dependency]);
                    constraints[indices[&dependency]].insert(indices[&key]);
                }
            }
        }
        // A public namespace cannot grow private export addresses. Keep local
        // readers with private declarations even when their accesses are deferred;
        // otherwise a pinned declaration can strand its relocated reader.
        let boundary_exports: BTreeMap<_, BTreeSet<_>> = surfaces
            .iter()
            .map(|&file| {
                (
                    file,
                    symbols.exports[&file]
                        .values()
                        .filter_map(|linked| match linked.identity {
                            ExportIdentity::Binding(file, binding) => Some((file, binding.0)),
                            _ => None,
                        })
                        .collect(),
                )
            })
            .collect();
        for access in &plan.retention.execution.accesses {
            if access.segment.file_id != access.declaration.file_id {
                continue;
            }
            let file = access.segment.file_id;
            let Some(exports) = boundary_exports.get(&seams[&file]) else {
                continue;
            };
            let reference = &modules[&file].parsed.facts.references[access.reference as usize];
            let Some(binding) = reference.binding else {
                continue;
            };
            if !exports.contains(&(file, binding.0)) {
                let before = indices[&access.declaration];
                let after = indices[&access.segment];
                constraints[before].insert(after);
                constraints[after].insert(before);
            }
        }
        let components = crate::effect_dependencies::components(&constraints);
        let mut component_for = BTreeMap::new();
        for (index, component) in components.iter().enumerate() {
            for &node in component {
                component_for.insert(keys[node], index);
            }
        }
        let movable: BTreeSet<_> = components
            .iter()
            .enumerate()
            .filter_map(|(i, nodes)| {
                (nodes.iter().any(|&n| declarations.contains(&keys[n]))
                    && nodes.iter().all(|&n| !pinned.contains(&keys[n])))
                .then_some(i)
            })
            .collect();
        let transparent: BTreeSet<_> = live
            .iter()
            .copied()
            .filter(|key| {
                movable.contains(&component_for[key])
                    || (!declarations.contains(key) && !effects.contains(key))
            })
            .collect();
        let mut users: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for edge in &plan.retention.edges {
            if edge.kind == RetentionEdgeKind::Binding {
                if let (GraphNode::Segment(from), GraphNode::Segment(to)) = (edge.from, edge.to) {
                    users.entry(to).or_default().push(from);
                }
            }
        }
        let mut public_users: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for &file in surfaces {
            for linked in symbols.exports[&file].values() {
                if let Some(key) = declaration(graph, &linked.identity)? {
                    public_users.entry(key).or_default().insert(seams[&file]);
                }
            }
        }
        for seed in plan.roots.values() {
            for node in &seed.binding_roots {
                if let GraphNode::Segment(key) = node {
                    public_users
                        .entry(*key)
                        .or_default()
                        .insert(seams[&key.file_id]);
                }
            }
        }
        let mut readonly_users: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for (&(file, binding), linked) in &symbols.imports {
            let facts = &modules[&file].parsed.facts;
            if facts.references.iter().any(|reference| {
                if !reference.write || reference.binding != Some(binding) {
                    return false;
                }
                let index = modules[&file]
                    .parsed
                    .segments
                    .partition_point(|segment| segment.span.end <= reference.span.start);
                live.contains(&SegmentKey::new(file, index as u32))
            }) {
                if let Some(key) = declaration(graph, &linked.identity)? {
                    readonly_users.entry(key).or_default().insert(seams[&file]);
                }
            }
        }
        let mut owners: BTreeMap<_, _> = live
            .iter()
            .map(|&k| (k, Owner::Evaluation(seams[&k.file_id])))
            .collect();
        let mut shared = BTreeMap::new();
        let mut candidates: Vec<_> = movable
            .iter()
            .map(|&i| {
                let mut component: Vec<_> = components[i].iter().map(|&n| keys[n]).collect();
                component.sort_by_key(|key| (graph.file_path(key.file_id), key.segment_idx));
                component
            })
            .collect();
        candidates.sort_by_key(|c| (graph.file_path(c[0].file_id), c[0].segment_idx));
        for component in &candidates {
            let mut reached = BTreeSet::new();
            let mut demand = BTreeSet::new();
            let mut pending = component.clone();
            while let Some(node) = pending.pop() {
                if !reached.insert(node) {
                    continue;
                }
                demand.extend(public_users.get(&node).into_iter().flatten().copied());
                for &user in users.get(&node).into_iter().flatten() {
                    if transparent.contains(&user) {
                        pending.push(user);
                    } else {
                        demand.insert(seams[&user.file_id]);
                    }
                }
            }
            let unique = demand
                .first()
                .filter(|_| demand.len() == 1)
                .copied()
                .filter(|owner| {
                    !cjs.contains(*owner)
                        && !wait_bound.contains(owner)
                        && !component.iter().any(|key| prefixes.contains(key))
                        && !component.iter().any(|key| {
                            readonly_users
                                .get(key)
                                .is_some_and(|users| users.contains(owner))
                        })
                });
            let owner = if let Some(owner) = unique {
                Owner::Evaluation(owner)
            } else {
                *shared
                    .entry(demand)
                    .or_insert(Owner::Bindings(component[0]))
            };
            for &key in component {
                owners.insert(key, owner);
            }
        }
        // Hoisting a function must not activate a later namespace/initializer
        // through its new chunk's transitive static imports. Index references
        // once per ownership revision, not once per candidate in a shared job.
        let mut anchored = BTreeSet::new();
        let mut activation = loop {
            let mut activation =
                ActivationIndex::new(graph, modules, symbols, seams, &owners, cjs)?;
            let mut rejected = Vec::new();
            for component in &candidates {
                let origin = Owner::Evaluation(seams[&component[0].file_id]);
                if owners[&component[0]] == origin {
                    continue;
                }
                if activation.summary(owners[&component[0]]).outside(origin) {
                    rejected.push(component);
                }
            }
            if rejected.is_empty() {
                break activation;
            }
            for component in rejected {
                anchored.insert(component[0]);
                for &key in component {
                    owners.insert(key, Owner::Evaluation(seams[&key.file_id]));
                }
            }
        };
        // Source-name-sensitive syntax stays lexical. Verify each proposed
        // physical group before applying any ownership change.
        loop {
            let mut rejected = BTreeSet::new();
            for (owner, members) in &activation.members {
                let names = local_names(graph, modules, symbols, &owners, *owner)?;
                let mut values = Values::new(prepared);
                values.names = names;
                values.requests = seams
                    .iter()
                    .map(|(&file, &owner)| (file, request(graph, owner)))
                    .collect();
                for (&symbol @ (file, binding), linked) in &symbols.imports {
                    if matches!(
                        linked.identity,
                        ExportIdentity::Namespace(_) | ExportIdentity::CommonJs(..)
                    ) {
                        let name = &modules[&file]
                            .parsed
                            .facts
                            .bindings
                            .iter()
                            .find(|b| b.id == binding)
                            .ok_or_eyre("missing namespace import binding")?
                            .name;
                        values.imports.insert(symbol, name.clone());
                    }
                }
                let group_live: BTreeSet<_> = members.iter().copied().collect();
                for file in group_live
                    .iter()
                    .map(|k| k.file_id)
                    .collect::<BTreeSet<_>>()
                {
                    if cjs.contains(file) {
                        continue;
                    }
                    if !compatible(graph, file, &modules[&file], &values, &group_live)? {
                        rejected.insert(*owner);
                    }
                }
            }
            if rejected.is_empty() {
                let placement = Self::scheduled(graph, plan, modules, owners.clone(), &prefixes)?;
                let (safe, initializers) = activation_safe(
                    graph, plan, modules, symbols, seams, surfaces, &placement, &prefixes,
                )?;
                if !safe {
                    // Reject contraction, not native cycles. Prefer retaining
                    // independent body-free groups while anchoring groups whose
                    // imports can introduce native evaluation activation.
                    let mut progress = false;
                    for component in &candidates {
                        if component.iter().any(|key| initializers.contains(key))
                            || (initializers.is_empty()
                                && component.iter().any(|key| {
                                    let module = &modules[&key.file_id];
                                    let span =
                                        module.parsed.segments[key.segment_idx as usize].span;
                                    module.parsed.facts.references.iter().any(|reference| {
                                        contains(span, reference.span)
                                            && reference.binding.is_some()
                                    })
                                }))
                        {
                            for &key in component {
                                let origin = Owner::Evaluation(seams[&key.file_id]);
                                progress |= owners[&key] != origin;
                                owners.insert(key, origin);
                            }
                            anchored.insert(component[0]);
                        }
                    }
                    if !progress {
                        for component in &candidates {
                            for &key in component {
                                let origin = Owner::Evaluation(seams[&key.file_id]);
                                progress |= owners[&key] != origin;
                                owners.insert(key, origin);
                            }
                            anchored.insert(component[0]);
                        }
                    }
                    eyre::ensure!(
                        progress,
                        "native activation proof failed without relocation"
                    );
                    activation =
                        ActivationIndex::new(graph, modules, symbols, seams, &owners, cjs)?;
                    continue;
                }
                return Ok(placement);
            }
            let mut progress = false;
            for component in &candidates {
                if !anchored.contains(&component[0]) && rejected.contains(&owners[&component[0]]) {
                    let owner = Owner::Bindings(component[0]);
                    for key in component {
                        progress |= owners[key] != owner;
                        owners.insert(*key, owner);
                    }
                }
            }
            eyre::ensure!(progress, "incompatible native lexical placement");
            activation = ActivationIndex::new(graph, modules, symbols, seams, &owners, cjs)?;
        }
    }
    fn scheduled(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        owners: BTreeMap<SegmentKey, Owner>,
        prefixes: &BTreeSet<SegmentKey>,
    ) -> eyre::Result<Self> {
        let mut dependencies: BTreeMap<_, BTreeSet<_>> =
            owners.keys().map(|&key| (key, BTreeSet::new())).collect();
        for constraint in &plan.retention.execution.constraints {
            if constraint.kind != ConstraintKind::EffectOrder {
                if owners[&constraint.before] != owners[&constraint.after] {
                    eyre::ensure!(
                        prefixes.contains(&constraint.before)
                            && matches!(owners[&constraint.before], Owner::Bindings(_))
                            && owners[&constraint.after]
                                == Owner::Evaluation(constraint.after.file_id),
                        "mandatory execution component crosses unchecked native owners"
                    );
                    continue;
                }
                dependencies
                    .get_mut(&constraint.after)
                    .ok_or_eyre("missing scheduled segment")?
                    .insert(constraint.before);
            }
        }
        // Pure relocated initializers use canonical eager accesses, not barrel
        // traversal. Cross-source phase barriers have already prevented movement.
        for access in &plan.retention.execution.accesses {
            if access.segment != access.declaration
                && owners[&access.segment] == owners[&access.declaration]
                && access.segment.file_id != access.declaration.file_id
            {
                let facts = &modules[&access.segment.file_id].parsed.facts;
                if facts.emission.execution.is_module_evaluation(
                    facts.emission.execution.references[access.reference as usize],
                ) {
                    dependencies
                        .get_mut(&access.segment)
                        .unwrap()
                        .insert(access.declaration);
                }
            }
        }
        // Preserve each remaining source's native statement order. This is local
        // to physical owners, never a global order between independent entries.
        let mut previous = BTreeMap::new();
        for (&key, &owner) in &owners {
            if let Some(before) = previous.insert((owner, key.file_id), key) {
                dependencies.get_mut(&key).unwrap().insert(before);
            }
        }
        let mut users: BTreeMap<_, Vec<_>> = BTreeMap::new();
        let mut ready = BTreeSet::new();
        for (&key, needs) in &dependencies {
            if needs.is_empty() {
                ready.insert((graph.file_path(key.file_id), key.segment_idx, key));
            }
            for &before in needs {
                users.entry(before).or_default().push(key);
            }
        }
        let mut schedules: BTreeMap<_, Vec<_>> = BTreeMap::new();
        while let Some((_, _, key)) = ready.pop_first() {
            schedules.entry(owners[&key]).or_default().push(key);
            for &user in users.get(&key).into_iter().flatten() {
                let needs = dependencies.get_mut(&user).unwrap();
                needs.remove(&key);
                if needs.is_empty() {
                    ready.insert((graph.file_path(user.file_id), user.segment_idx, user));
                }
            }
        }
        eyre::ensure!(
            schedules.values().map(Vec::len).sum::<usize>() == owners.len(),
            "mandatory native segment schedule contains a cycle"
        );
        let phase_dependencies = phase_dependencies(&owners, prefixes);
        Ok(Self {
            owners,
            schedules,
            phase_dependencies,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct ActivationSummary {
    phase: Option<Owner>,
    conflicting: bool,
}

impl ActivationSummary {
    fn include(&mut self, phase: Owner) {
        self.conflicting |= self.phase.is_some_and(|previous| previous != phase);
        self.phase = Some(phase);
    }

    fn outside(self, origin: Owner) -> bool {
        self.conflicting || self.phase.is_some_and(|phase| phase != origin)
    }
}

#[derive(Default)]
struct ActivationIndex {
    members: BTreeMap<Owner, Vec<SegmentKey>>,
    references: BTreeMap<Owner, BTreeSet<Owner>>,
    namespaces: BTreeMap<Owner, ActivationSummary>,
    opaque: BTreeSet<Owner>,
    summaries: BTreeMap<Owner, ActivationSummary>,
    #[cfg(test)]
    visits: usize,
    #[cfg(test)]
    indexed_segments: usize,
}

impl ActivationIndex {
    fn from_owners(owners: &BTreeMap<SegmentKey, Owner>) -> Self {
        let mut index = Self::default();
        for (&key, &owner) in owners {
            index.members.entry(owner).or_default().push(key);
            #[cfg(test)]
            {
                index.indexed_segments += 1;
            }
        }
        index
    }

    fn new(
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        seams: &BTreeMap<u32, u32>,
        owners: &BTreeMap<SegmentKey, Owner>,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<Self> {
        let mut index = Self::from_owners(owners);
        for (&file, module) in modules {
            for reference in &module.parsed.facts.references {
                let segment = module
                    .parsed
                    .segments
                    .partition_point(|s| s.span.end <= reference.span.start);
                let key = SegmentKey::new(file, segment as u32);
                let Some(&owner) = owners.get(&key) else {
                    continue;
                };
                let Some(binding) = reference.binding else {
                    continue;
                };
                let identity = if symbols.definitions.contains_key(&(file, binding)) {
                    ExportIdentity::Binding(file, binding)
                } else if let Some(linked) = symbols.imports.get(&(file, binding)) {
                    linked.identity.clone()
                } else {
                    continue;
                };
                let target = if let Some(key) = declaration(graph, &identity)? {
                    Some(owners[&key])
                } else if let ExportIdentity::Namespace(file) = identity {
                    if cjs.contains(file) {
                        index.opaque.insert(owner);
                    }
                    index
                        .namespaces
                        .entry(owner)
                        .or_default()
                        .include(Owner::Evaluation(seams[&file]));
                    None
                } else {
                    if matches!(identity, ExportIdentity::CommonJs(..)) {
                        index.opaque.insert(owner);
                    }
                    None
                };
                if let Some(target) = target {
                    index.references.entry(owner).or_default().insert(target);
                }
            }
        }
        Ok(index)
    }

    fn summary(&mut self, start: Owner) -> ActivationSummary {
        if let Some(&summary) = self.summaries.get(&start) {
            return summary;
        }
        let mut summary = ActivationSummary::default();
        let mut pending = vec![start];
        let mut seen = BTreeSet::new();
        while let Some(owner) = pending.pop() {
            if !seen.insert(owner) {
                continue;
            }
            #[cfg(test)]
            {
                self.visits += 1;
            }
            if owner != start && matches!(owner, Owner::Evaluation(_)) {
                summary.include(owner);
                continue;
            }
            summary.conflicting |= self.opaque.contains(&owner);
            if let Some(namespace) = self.namespaces.get(&owner) {
                summary.conflicting |= namespace.conflicting;
                if let Some(phase) = namespace.phase {
                    summary.include(phase);
                }
            }
            pending.extend(self.references.get(&owner).into_iter().flatten().copied());
        }
        self.summaries.insert(start, summary);
        summary
    }
}

#[cfg(test)]
mod activation_tests {
    use super::*;

    #[test]
    fn sparse_shared_leaves_reuse_owner_activation_analysis() {
        for count in [32, 1024] {
            let shared = Owner::Bindings(SegmentKey::new(0, 0));
            let owners = (0..count)
                .map(|file| (SegmentKey::new(file, 0), shared))
                .collect();
            let mut index = ActivationIndex::from_owners(&owners);
            assert_eq!(index.members[&shared].len(), count as usize);
            for file in 0..count {
                assert!(!index.summary(shared).outside(Owner::Evaluation(file)));
            }
            assert_eq!(index.visits, 1, "one traversal, not one per leaf");
            assert_eq!(
                index.indexed_segments, count as usize,
                "one membership pass"
            );
            assert_eq!(index.summaries.len(), 1);
        }
    }

    #[test]
    fn cached_activation_summary_still_checks_each_origin() {
        let shared = Owner::Bindings(SegmentKey::new(0, 0));
        let indirect = Owner::Bindings(SegmentKey::new(1, 0));
        let phase = Owner::Evaluation(2);
        let mut index = ActivationIndex {
            references: BTreeMap::from([
                (shared, BTreeSet::from([indirect])),
                (indirect, BTreeSet::from([shared, phase])),
            ]),
            ..ActivationIndex::default()
        };
        assert!(!index.summary(shared).outside(phase));
        assert!(index.summary(shared).outside(Owner::Evaluation(3)));
        assert_eq!(index.visits, 3);
    }

    #[test]
    fn namespace_activation_keeps_the_original_source_phase_guard() {
        let consumer = Owner::Evaluation(0);
        let origin = Owner::Evaluation(1);
        let mut index = ActivationIndex {
            references: BTreeMap::from([(consumer, BTreeSet::from([consumer]))]),
            ..ActivationIndex::default()
        };
        assert!(!index.summary(consumer).outside(origin));
        index
            .namespaces
            .entry(consumer)
            .or_default()
            .include(consumer);
        index.summaries.clear();
        assert!(index.summary(consumer).outside(origin));
        assert!(!index.summary(consumer).outside(consumer));
    }
}

fn phase_dependencies(
    owners: &BTreeMap<SegmentKey, Owner>,
    prefixes: &BTreeSet<SegmentKey>,
) -> BTreeMap<Owner, BTreeSet<Owner>> {
    let mut dependencies: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for key in prefixes {
        let origin = Owner::Evaluation(key.file_id);
        if owners[key] != origin {
            dependencies.entry(origin).or_default().insert(owners[key]);
        }
    }
    dependencies
}

/// Compare initialization visibility at every native body event, independently
/// for each entrance. Pure initializers may commute only between those events:
/// a throw must expose exactly the same initialized lexical bindings.
#[expect(clippy::too_many_arguments)]
fn activation_safe(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    seams: &BTreeMap<u32, u32>,
    surfaces: &BTreeSet<u32>,
    placement: &Placement,
    prefixes: &BTreeSet<SegmentKey>,
) -> eyre::Result<(bool, BTreeSet<SegmentKey>)> {
    let owners = &placement.owners;
    let mut native = BTreeMap::new();
    let mut physical: BTreeMap<Owner, Vec<Owner>> =
        owners.values().map(|&owner| (owner, Vec::new())).collect();
    for (&file, module) in modules {
        let mut dependencies = Vec::new();
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .filter(|dependency| {
                !dependency.type_only
                    && matches!(
                        dependency.kind,
                        DependencyKind::Import | DependencyKind::ReExport
                    )
            })
        {
            dependencies.push(target_file(graph, file, dependency)?);
        }
        physical
            .entry(Owner::Evaluation(seams[&file]))
            .or_default()
            .extend(
                dependencies
                    .iter()
                    .filter(|&&target| seams[&target] != seams[&file])
                    .map(|target| Owner::Evaluation(seams[target])),
            );
        native.insert(file, dependencies);
    }
    for (owner, dependencies) in phase_dependencies(owners, prefixes) {
        let mut dependencies: Vec<_> = dependencies.into_iter().collect();
        dependencies.sort_by_key(|owner| owner.path(graph));
        physical.entry(owner).or_default().extend(dependencies);
    }
    // Match emitter ordering: original evaluation requests first, then canonical
    // lexical imports in source-path/binding order, then public reexports.
    for (&owner, schedule) in &placement.schedules {
        let mut files: Vec<_> = schedule
            .iter()
            .map(|key| key.file_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        files.sort_by_key(|&file| graph.file_path(file));
        for file in files {
            let module = &modules[&file];
            for binding in module
                .parsed
                .facts
                .bindings
                .iter()
                .filter(|binding| binding.top_level)
            {
                if !module.parsed.facts.references.iter().any(|reference| {
                    if reference.binding != Some(binding.id) {
                        return false;
                    }
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|segment| segment.span.end <= reference.span.start);
                    owners.get(&SegmentKey::new(file, index as u32)) == Some(&owner)
                }) {
                    continue;
                }
                let identity = if symbols.definitions.contains_key(&(file, binding.id)) {
                    ExportIdentity::Binding(file, binding.id)
                } else if let Some(linked) = symbols.imports.get(&(file, binding.id)) {
                    linked.identity.clone()
                } else {
                    continue;
                };
                let target = match declaration(graph, &identity)? {
                    Some(key) => Some(owners[&key]),
                    None => match identity {
                        ExportIdentity::Namespace(file) | ExportIdentity::CommonJs(file, _) => {
                            Some(Owner::Evaluation(seams[&file]))
                        }
                        _ => None,
                    },
                };
                if let Some(target) = target.filter(|&target| target != owner) {
                    physical.entry(owner).or_default().push(target);
                }
            }
        }
        if let Owner::Evaluation(file) = owner {
            if surfaces.contains(&file) {
                for linked in symbols.exports[&file].values() {
                    if let Some(key) = declaration(graph, &linked.identity)? {
                        if owners[&key] != owner {
                            physical.entry(owner).or_default().push(owners[&key]);
                        }
                    }
                }
            }
        }
    }
    fn completion<K: Copy + Ord>(root: K, edges: &BTreeMap<K, Vec<K>>) -> Vec<K> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![(root, false)];
        let mut order = Vec::new();
        while let Some((node, finish)) = pending.pop() {
            if finish {
                order.push(node);
                continue;
            }
            if !seen.insert(node) {
                continue;
            }
            pending.push((node, true));
            pending.extend(
                edges
                    .get(&node)
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|&target| (target, false)),
            );
        }
        order
    }
    let effects: BTreeSet<_> = plan
        .retention
        .effects
        .iter()
        .filter(|effect| effect.summary.is_effectful())
        .map(|effect| effect.key)
        .collect();
    let mut lexical = BTreeSet::new();
    let mut events = BTreeSet::new();
    let mut native_events: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for &key in owners.keys() {
        let recipe =
            &modules[&key.file_id].parsed.facts.emission.segments[key.segment_idx as usize];
        if recipe.connector_only || recipe.placement.instantiable {
            continue;
        }
        events.insert(key);
        native_events.entry(key.file_id).or_default().push(key);
        if !effects.contains(&key)
            && (!recipe.placement.declarations.is_empty() || recipe.placement.default_expression)
        {
            lexical.insert(key);
        }
    }
    let projection = |order: Vec<SegmentKey>| {
        let mut initialized = BTreeSet::new();
        let mut phases = Vec::new();
        for key in order {
            if lexical.contains(&key) {
                initialized.insert(key);
            } else if events.contains(&key) {
                phases.push((key, std::mem::take(&mut initialized)));
            }
        }
        // After the last observable/throwing event, independent pure work may
        // belong only to another entrance. Binding imports still initialize
        // every demanded public value before that entrance completes.
        phases
    };
    for seed in plan.roots.values() {
        let mut entrances = seed.evaluation_roots.clone();
        entrances.extend(seed.binding_roots.iter().filter_map(|node| match node {
            GraphNode::Module(file) => Some(*file),
            GraphNode::Segment(key) => Some(key.file_id),
            GraphNode::Unlinked { .. } => None,
        }));
        for file in entrances {
            let before = projection(
                completion(file, &native)
                    .into_iter()
                    .flat_map(|file| native_events.get(&file).into_iter().flatten().copied())
                    .collect(),
            );
            let after = projection(
                completion(Owner::Evaluation(seams[&file]), &physical)
                    .into_iter()
                    .flat_map(|owner| placement.schedule(owner).iter().copied())
                    .collect(),
            );
            if before != after {
                let mut changed = BTreeSet::new();
                for ((before_phase, before_init), (after_phase, after_init)) in
                    before.iter().zip(&after)
                {
                    if before_phase != after_phase {
                        return Ok((false, BTreeSet::new()));
                    }
                    changed.extend(before_init.symmetric_difference(after_init).copied());
                }
                return Ok((false, changed));
            }
        }
    }
    Ok((true, BTreeSet::new()))
}

fn declaration(graph: &SourceGraph, identity: &ExportIdentity) -> eyre::Result<Option<SegmentKey>> {
    Ok(match identity {
        ExportIdentity::Binding(file, binding) => {
            let facts = graph.file_facts(*file).ok_or_eyre("missing symbol facts")?;
            let binding = facts
                .bindings
                .iter()
                .find(|b| b.id == *binding)
                .ok_or_eyre("missing symbol binding")?;
            let span = *binding
                .declarations
                .first()
                .ok_or_eyre("missing symbol declaration")?;
            let segments = graph
                .file_segments(*file)
                .ok_or_eyre("missing symbol segments")?;
            Some(SegmentKey::new(
                *file,
                segments.partition_point(|s| s.span.end <= span.start) as u32,
            ))
        }
        ExportIdentity::Expression(key) => Some(*key),
        _ => None,
    })
}

pub(super) fn local_names(
    graph: &SourceGraph,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    owners: &BTreeMap<SegmentKey, Owner>,
    owner: Owner,
) -> eyre::Result<HashMap<crate::symbols::Symbol, String>> {
    let files: BTreeSet<_> = owners
        .iter()
        .filter_map(|(key, &assigned)| (assigned == owner).then_some(key.file_id))
        .collect();
    let mut occupied: BTreeSet<_> = files
        .iter()
        .flat_map(|file| {
            modules[file]
                .parsed
                .facts
                .emission
                .free_references
                .iter()
                .map(|&index| {
                    modules[file].parsed.facts.references[index as usize]
                        .name
                        .clone()
                })
        })
        .collect();
    let mut definitions = Vec::new();
    for &(file, binding) in symbols.definitions.keys() {
        if declaration(graph, &ExportIdentity::Binding(file, binding))?
            .is_some_and(|key| owners.get(&key) == Some(&owner))
        {
            definitions.push((file, binding));
        } else if modules[&file]
            .parsed
            .facts
            .references
            .iter()
            .any(|reference| {
                if reference.binding != Some(binding) {
                    return false;
                }
                let index = modules[&file]
                    .parsed
                    .segments
                    .partition_point(|segment| segment.span.end <= reference.span.start);
                owners.get(&SegmentKey::new(file, index as u32)) == Some(&owner)
            })
        {
            occupied.insert(symbols.definitions[&(file, binding)].name.clone());
        }
    }
    definitions.sort_by_key(|&(file, binding)| (graph.file_path(file), binding.0));
    let mut names = HashMap::new();
    for symbol @ (file, binding) in definitions {
        let original = modules[&file]
            .parsed
            .facts
            .bindings
            .iter()
            .find(|b| b.id == binding)
            .ok_or_eyre("missing relocated symbol binding")?;
        let exported = symbols.exports[&file]
            .values()
            .any(|export| export.identity == ExportIdentity::Binding(file, binding));
        let captured = exported
            && files.iter().any(|&other| {
                other != file
                    && modules[&other]
                        .parsed
                        .facts
                        .emission
                        .nested_bindings
                        .iter()
                        .any(|id| {
                            modules[&other]
                                .parsed
                                .facts
                                .bindings
                                .iter()
                                .any(|b| b.id == *id && b.name == original.name)
                        })
            });
        let name = if !captured && occupied.insert(original.name.clone()) {
            original.name.clone()
        } else {
            let name = symbols.definitions[&symbol].name.clone();
            occupied.insert(name.clone());
            name
        };
        names.insert(symbol, name);
    }
    Ok(names)
}

#[expect(clippy::too_many_arguments)]
pub(super) fn emit(
    graph: &SourceGraph,
    root: Owner,
    seams: &BTreeMap<u32, u32>,
    redirects: &BTreeMap<u32, u32>,
    modules: &BTreeMap<u32, Module<'_>>,
    prepared: &PreparedValues<'_>,
    placement: &Placement,
    surfaces: &BTreeSet<u32>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
    cjs: &commonjs::CommonJs,
) -> eyre::Result<Code> {
    let symbols = prepared.symbols;
    let live: BTreeSet<_> = placement
        .owners
        .iter()
        .filter(|(_, &o)| o == root)
        .map(|(&k, _)| k)
        .collect();
    let mut values = Values::new(prepared);
    values.names = local_names(graph, modules, symbols, &placement.owners, root)?;
    values.requests = seams
        .iter()
        .map(|(&f, &o)| (f, Code::request(path(graph, o), RequestKind::DynamicImport)))
        .collect();
    for (&file, module) in modules {
        for dependency in module.parsed.facts.dependencies.iter().filter(|d| {
            !d.type_only
                && d.kind == DependencyKind::DynamicImport
                && live.iter().any(|key| {
                    key.file_id == file
                        && contains(
                            module.parsed.segments[key.segment_idx as usize].span,
                            d.span,
                        )
                })
        }) {
            if let Some(dispatch) = cjs.context_dynamic(graph, file, dependency, seams)? {
                values.contexts.insert((file, dependency.id), dispatch);
                continue;
            }
            let target = target_file(graph, file, dependency)?;
            if cjs.contains(target) {
                values.dynamic.insert(target, cjs.dynamic(graph, target)?);
            }
        }
    }
    let mut output = Code::default();
    if let Owner::Evaluation(file) = root {
        if let Some(name) = values.defaults.iter().find_map(|(key, name)| {
            (key.file_id == file && placement.owners.get(key) == Some(&root)).then_some(name)
        }) {
            output.push(
                Code::request(root.path(graph), RequestKind::Import)
                    .surrounded(format!("import {{default as {name}}} from "), ";\n"),
            );
            values.native_defaults.insert(file);
        }
    }
    if let Owner::Evaluation(file) = root {
        for dependency in modules[&file].parsed.facts.dependencies.iter().filter(|d| {
            !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
        }) {
            let target = redirects[&target_file(graph, file, dependency)?];
            if cjs.contains(target) {
                output.push(cjs.evaluate(graph, file, target)?);
            } else if seams[&target] != file {
                output.push(Code::evaluation(path(graph, seams[&target])));
            }
        }
    }
    let mut phase_dependencies: Vec<_> = placement
        .phase_dependencies
        .get(&root)
        .into_iter()
        .flatten()
        .copied()
        .collect();
    phase_dependencies.sort_by_key(|owner| owner.path(graph));
    for owner in phase_dependencies {
        output.push(Code::evaluation(owner.path(graph)));
    }
    let schedule = placement.schedule(root);
    let mut relocated: Vec<_> = schedule
        .iter()
        .map(|key| key.file_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    relocated.sort_by_key(|&file| graph.file_path(file));
    let mut imported = BTreeSet::new();
    let mut occupied = OccupiedNames::new(prepared);
    for name in values.names.values() {
        occupied.insert(name.clone());
    }
    for &file in &relocated {
        for binding in modules[&file]
            .parsed
            .facts
            .bindings
            .iter()
            .filter(|b| b.top_level)
        {
            if !modules[&file].parsed.facts.references.iter().any(|r| {
                r.binding == Some(binding.id)
                    && live.iter().filter(|k| k.file_id == file).any(|k| {
                        contains(
                            modules[&file].parsed.segments[k.segment_idx as usize].span,
                            r.span,
                        )
                    })
            }) {
                continue;
            }
            let identity = if symbols.definitions.contains_key(&(file, binding.id)) {
                ExportIdentity::Binding(file, binding.id)
            } else if let Some(linked) = symbols.imports.get(&(file, binding.id)) {
                linked.identity.clone()
            } else {
                continue;
            };
            let owner = declaration(graph, &identity)?.map(|key| placement.owners[&key]);
            let readonly = binding.import
                && modules[&file]
                    .parsed
                    .facts
                    .references
                    .iter()
                    .any(|r| r.binding == Some(binding.id) && r.write);
            if owner == Some(root) && !readonly {
                continue;
            }
            let alias = symbols
                .definitions
                .get(&(file, binding.id))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| {
                    let mut alias = format!(
                        "__gf_segment_import_{}_{}",
                        digest(graph.file_path(file).unwrap().to_str().unwrap()),
                        binding.id.0
                    );
                    while !occupied.insert(alias.clone()) {
                        alias.push('_');
                    }
                    alias
                });
            if imported.insert(alias.clone()) {
                match &identity {
                    ExportIdentity::Namespace(target) if cjs.contains(*target) => {
                        output.push(cjs.import(
                            graph,
                            *target,
                            &ImportedName::Namespace,
                            &alias,
                        )?);
                    }
                    ExportIdentity::Namespace(target) => output.push(
                        Code::request(path(graph, seams[target]), RequestKind::Import)
                            .surrounded(format!("import * as {alias} from "), ";\n"),
                    ),
                    ExportIdentity::CommonJs(target, name) => {
                        output.push(cjs.import(
                            graph,
                            *target,
                            &ImportedName::Name(name.clone()),
                            &alias,
                        )?);
                    }
                    _ => {
                        let owner = owner.ok_or_eyre("missing lexical segment owner")?;
                        output.push(
                            Code::request(owner.path(graph), RequestKind::Import).surrounded(
                                format!(
                                    "import {{{} as {alias}}} from ",
                                    quote(&address(graph, &identity, owner, surfaces, &values)?)
                                ),
                                ";\n",
                            ),
                        );
                    }
                }
            }
            values.imports.insert((file, binding.id), alias);
        }
    }
    let mut bodies = Code::default();
    for &key in schedule {
        let file = key.file_id;
        let (hoisted, body) = body(graph, key, &modules[&file], &values, recipes)?
            .ok_or_eyre("relocated lexical group changed compatibility")?;
        output.push(hoisted);
        bodies.push(body);
        bodies.push_str("\n;\n");
    }
    output.push(bodies);
    let mut exported = BTreeSet::new();
    let public_exports = match root {
        Owner::Evaluation(file) => symbols.exports.get(&file),
        Owner::Bindings(_) => None,
    };
    let public = matches!(root, Owner::Evaluation(file) if surfaces.contains(&file));
    for (public_name, linked) in public_exports.into_iter().flatten() {
        if public_name == "default"
            && matches!(linked.identity, ExportIdentity::Expression(key) if values.native_defaults.contains(&key.file_id))
        {
            exported.insert(public_name.clone());
            continue;
        }
        if let ExportIdentity::Namespace(file) = linked.identity {
            if public {
                if cjs.contains(file) {
                    output.push(cjs.reexport(
                        graph,
                        file,
                        &ImportedName::Namespace,
                        public_name,
                    )?);
                } else {
                    output.push(
                        Code::request(path(graph, seams[&file]), RequestKind::ReExport)
                            .surrounded(format!("export * as {} from ", quote(public_name)), ";\n"),
                    );
                }
                exported.insert(public_name.clone());
            }
            continue;
        }
        if let ExportIdentity::CommonJs(file, ref name) = linked.identity {
            if public {
                output.push(cjs.reexport(
                    graph,
                    file,
                    &ImportedName::Name(name.clone()),
                    public_name,
                )?);
                exported.insert(public_name.clone());
            }
            continue;
        }
        let key = declaration(graph, &linked.identity)?
            .ok_or_eyre("nonlexical relocated public export")?;
        let owner = placement.owners[&key];
        if owner == root {
            output.push_str(&format!(
                "export {{{} as {}}};\n",
                values
                    .name(&linked.identity)
                    .ok_or_eyre("missing relocated export")?,
                quote(public_name)
            ));
            exported.insert(public_name.clone());
        } else if public {
            output.push(
                Code::request(owner.path(graph), RequestKind::ReExport).surrounded(
                    format!(
                        "export {{{} as {}}} from ",
                        quote(&address(graph, &linked.identity, owner, surfaces, &values)?),
                        quote(public_name)
                    ),
                    ";\n",
                ),
            );
            exported.insert(public_name.clone());
        }
    }
    if !public {
        let mut definitions: Vec<_> = symbols.definitions.iter().collect();
        definitions.sort_by_key(|(&(file, binding), _)| (graph.file_path(file), binding.0));
        for (&symbol @ (file, binding), definition) in definitions {
            let identity = ExportIdentity::Binding(file, binding);
            if declaration(graph, &identity)?.is_some_and(|k| placement.owners[&k] == root) {
                let address = address(graph, &identity, root, surfaces, &values)?;
                if !exported.insert(address.clone()) {
                    continue;
                }
                let name = values.names.get(&symbol).unwrap_or(&definition.name);
                output.push_str(&format!("export {{{name} as {}}};\n", quote(&address)));
            }
        }
        for (key, name) in values.defaults {
            if placement.owners.get(key) == Some(&root) {
                let address = address(
                    graph,
                    &ExportIdentity::Expression(*key),
                    root,
                    surfaces,
                    &values,
                )?;
                if exported.insert(address.clone()) {
                    output.push_str(&format!("export {{{name} as {}}};\n", quote(&address)));
                }
            }
        }
    }
    output.default_export = exported.contains("default");
    Ok(output)
}

fn address(
    graph: &SourceGraph,
    identity: &ExportIdentity,
    owner: Owner,
    surfaces: &BTreeSet<u32>,
    values: &Values<'_>,
) -> eyre::Result<String> {
    let symbols = values.symbols;
    let original_exports = match owner {
        Owner::Evaluation(file) => symbols.exports.get(&file),
        Owner::Bindings(_) => None,
    };
    if let Some(exports) = original_exports {
        if let Some((name, _)) = exports.iter().find(|(_, e)| &e.identity == identity) {
            return Ok(name.clone());
        }
    }
    eyre::ensure!(
        !matches!(owner, Owner::Evaluation(file) if surfaces.contains(&file)),
        "missing boundary symbol export"
    );
    match identity {
        ExportIdentity::Binding(file, binding) => {
            let original = &symbols.definitions[&(*file, *binding)].name;
            if !original_exports.is_some_and(|exports| exports.contains_key(original)) {
                return Ok(original.clone());
            }
            let mut name = format!(
                "__gf_segment_binding_{}_{}",
                digest(graph.file_path(*file).unwrap().to_str().unwrap()),
                binding.0
            );
            while original_exports.is_some_and(|exports| exports.contains_key(&name))
                || symbols.definitions.values().any(|d| d.name == name)
                || values.defaults.values().any(|n| *n == name)
            {
                name.push('_');
            }
            Ok(name)
        }
        ExportIdentity::Expression(key) => values
            .defaults
            .get(key)
            .cloned()
            .ok_or_eyre("missing relocated expression address"),
        _ => eyre::bail!("unsupported nonlexical relocated address"),
    }
}
