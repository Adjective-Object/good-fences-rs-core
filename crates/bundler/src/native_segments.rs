//! Physical ownership for pure declarations, independent of source body seams.
use super::*;
use crate::assembly::{Code, RequestKind};
use crate::execution_dependencies::{BarrierReason, ConstraintKind};
use crate::RetentionEdgeKind;

pub(super) struct Placement {
    pub owners: BTreeMap<SegmentKey, Owner>,
    pub names: BTreeMap<Owner, connections::OwnerNames>,
    schedules: BTreeMap<Owner, Vec<SegmentKey>>,
    prefixes: BTreeSet<SegmentKey>,
    pub(super) phase_dependencies: BTreeMap<Owner, BTreeSet<Owner>>,
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
    fn generated_islands(
        graph: &SourceGraph,
        plan: &ChunkPlan,
    ) -> eyre::Result<Vec<BTreeSet<SegmentKey>>> {
        let mut islands: Vec<BTreeSet<SegmentKey>> = Vec::new();
        for boundary in plan.generated_boundaries.values() {
            let mut island = BTreeSet::new();
            let mut pending = vec![boundary.target];
            while let Some(key) = pending.pop() {
                if !island.insert(key) {
                    continue;
                }
                eyre::ensure!(
                    matches!(
                        graph
                            .file_facts(key.file_id)
                            .unwrap()
                            .emission()
                            .segments()
                            .get(key.segment_idx as usize)
                            .unwrap()
                            .deferred(),
                        ast_segmenter::views::DeferredForm::Function
                    ),
                    "nonfunction generated island"
                );
                for edge in &plan.retention.edges {
                    if edge.kind == RetentionEdgeKind::Binding
                        && edge.from == GraphNode::Segment(key)
                    {
                        if let GraphNode::Segment(target) = edge.to {
                            pending.push(target);
                        }
                    }
                }
            }
            let mut index = 0;
            while index < islands.len() {
                if !islands[index].is_disjoint(&island) {
                    island.extend(islands.remove(index));
                } else {
                    index += 1;
                }
            }
            islands.push(island);
        }
        Ok(islands)
    }

    pub(super) fn extract_generated(
        &mut self,
        graph: &SourceGraph,
        plan: &ChunkPlan,
    ) -> eyre::Result<()> {
        for island in Self::generated_islands(graph, plan)? {
            let first = *island
                .iter()
                .min_by_key(|k| (graph.file_path(k.file_id), k.segment_idx))
                .unwrap();
            for key in island {
                self.owners.insert(key, Owner::Bindings(first));
            }
        }
        Ok(())
    }

    pub(super) fn reschedule(
        &mut self,
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        prepared: &PreparedValues<'_>,
        seams: &BTreeMap<u32, u32>,
    ) -> eyre::Result<()> {
        let mut scheduled =
            Self::scheduled(graph, plan, modules, self.owners.clone(), &self.prefixes)?;
        let definitions = partition_definitions(graph, prepared, &scheduled.owners)?;
        scheduled.names = connections::name_plan(
            graph,
            plan,
            modules,
            prepared,
            &scheduled.owners,
            seams,
            &definitions,
        )?;
        *self = scheduled;
        Ok(())
    }

    pub fn schedule(&self, owner: Owner) -> &[SegmentKey] {
        self.schedules
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn hoist_generated(
        &mut self,
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        target: usize,
    ) -> eyre::Result<()> {
        if target == 0 {
            return Ok(());
        }
        for island in Self::generated_islands(graph, plan)? {
            let owner = self.owners[island.first().unwrap()];
            // Never split a library produced by the ordinary mask-based clustering.
            if self
                .owners
                .iter()
                .any(|(key, &assigned)| assigned == owner && !island.contains(key))
            {
                continue;
            }
            let bytes: usize = island
                .iter()
                .map(|key| {
                    modules[&key.file_id]
                        .parsed
                        .facts()
                        .emission()
                        .segments()
                        .get(key.segment_idx as usize)
                        .unwrap()
                        .source()
                        .spans()
                        .iter()
                        .map(|s| (s.end - s.start) as usize)
                        .sum::<usize>()
                })
                .sum();
            if bytes >= target {
                continue;
            }
            let parents: BTreeSet<_> = plan
                .generated_boundaries
                .values()
                .filter(|b| island.contains(&b.target))
                .map(|b| match graph.edges()[b.source_edge].from {
                    GraphNode::Segment(key) => self.owners[&key],
                    _ => unreachable!(),
                })
                .collect();
            if let Some(&parent @ Owner::Evaluation(file)) =
                parents.first().filter(|_| parents.len() == 1)
            {
                if island.iter().all(|key| key.file_id == file) {
                    for key in island {
                        self.owners.insert(key, parent);
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn cluster(
        &mut self,
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        prepared: &PreparedValues<'_>,
        seams: &BTreeMap<u32, u32>,
        target: usize,
    ) -> eyre::Result<()> {
        if target == 0 {
            return Ok(());
        }
        let mut groups = BTreeMap::<Owner, BTreeSet<SegmentKey>>::new();
        for (&key, &owner) in &self.owners {
            if matches!(owner, Owner::Bindings(_)) {
                groups.entry(owner).or_default().insert(key);
            }
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by_key(|(_, keys)| {
            keys.iter()
                .map(|k| (graph.file_path(k.file_id).unwrap(), k.segment_idx))
                .min()
        });
        let mut eligible = Vec::new();
        let mut inputs = Vec::new();
        for (owner, keys) in groups {
            let mask: BTreeSet<_> = keys
                .iter()
                .flat_map(|&key| {
                    plan.node_roots
                        .get(&crate::chunks::node_id(graph, GraphNode::Segment(key)).unwrap())
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect();
            let bytes = keys
                .iter()
                .map(|key| {
                    modules[&key.file_id]
                        .parsed
                        .facts()
                        .emission()
                        .segments()
                        .get(key.segment_idx as usize)
                        .unwrap()
                        .source()
                        .spans()
                        .iter()
                        .map(|s| (s.end - s.start) as usize)
                        .sum::<usize>()
                })
                .sum();
            if !mask.is_empty() && bytes < target {
                eligible.push((owner, keys));
                inputs.push((mask, bytes));
            }
        }
        self.owners = libraries::select(
            &inputs,
            target,
            self.owners.clone(),
            |current, members, next| {
                if inputs[next]
                    .0
                    .iter()
                    .any(|root| matches!(root, RootId::AsyncExport { .. }))
                    || members.iter().any(|&member| {
                        inputs[member]
                            .0
                            .iter()
                            .any(|root| matches!(root, RootId::AsyncExport { .. }))
                    })
                {
                    return Ok(None);
                }
                let owner = eligible[*members.first().unwrap()].0;
                let mut proposed = current.clone();
                for &key in &eligible[next].1 {
                    proposed.insert(key, owner);
                }
                // A pure library must not import an evaluation job on another member's behalf.
                // Reject new import cycles as well: they could expose eager-read TDZs.
                if !binding_closure_safe(graph, modules, prepared.symbols, &proposed, owner)? {
                    return Ok(None);
                }
                let mut values = Values::new(prepared);
                let definitions = partition_definitions(graph, prepared, &proposed)?;
                let names = connections::name_plan(
                    graph,
                    plan,
                    modules,
                    prepared,
                    &proposed,
                    seams,
                    &definitions,
                )?;
                values.names = names[&owner].locals.clone();
                for binding in &names[&owner].bindings {
                    values.imports.insert(binding.symbol, binding.alias.clone());
                }
                values.requests = seams
                    .iter()
                    .map(|(&f, &o)| (f, request(graph, o)))
                    .collect();
                let live: BTreeSet<_> = proposed
                    .iter()
                    .filter_map(|(&k, &o)| (o == owner).then_some(k))
                    .collect();
                for file in live.iter().map(|k| k.file_id).collect::<BTreeSet<_>>() {
                    if !compatible(graph, file, &modules[&file], &values, &live)? {
                        return Ok(None);
                    }
                }
                Ok(Some(proposed))
            },
        )?;
        Ok(())
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
            let facts = &module.parsed.facts();
            // Local immutable reads after initialization cannot invoke user code
            // or throw. Mandatory components still carry their exact ordering.
            let safe_reads = !facts.commonjs()
                && facts.top_level_await().is_empty()
                && facts
                    .references()
                    .iter()
                    .all(|reference| !reference.write())
                && facts
                    .emission()
                    .segments()
                    .iter()
                    .all(|recipe| recipe.placement().forward_references().is_empty())
                && reasons.iter().all(|reason| {
                    matches!(
                        reason,
                        BarrierReason::AbruptCompletion | BarrierReason::DeferredAccess
                    )
                })
                && facts.effects().iter().all(|site| {
                    if !facts
                        .emission()
                        .execution()
                        .effect_is_module_evaluation(site)
                        .unwrap_or(true)
                    {
                        return true;
                    }
                    site.reason() == ast_segmenter::effects::EffectReason::BindingRead
                        && facts
                            .references()
                            .iter()
                            .filter(|reference| reference.span() == site.span())
                            .any(|reference| {
                                reference
                                    .binding()
                                    .and_then(|id| {
                                        facts.bindings().iter().find(|binding| {
                                            binding.id() == id
                                                && !binding.import()
                                                && binding.top_level()
                                        })
                                    })
                                    .is_some_and(|binding| {
                                        binding.declarations().first().is_some_and(|span| {
                                            let index = module.parsed.segments().partition_point(
                                                |segment| segment.span.end <= span.start,
                                            );
                                            module.parsed.segments().get(index).unwrap().span.end
                                                <= reference.span().start
                                                || facts
                                                    .emission()
                                                    .segments()
                                                    .get(index)
                                                    .unwrap()
                                                    .placement()
                                                    .hoisted()
                                                    .contains(&binding.id())
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
                let placement = modules[&key.file_id]
                    .parsed
                    .facts()
                    .emission()
                    .segments()
                    .get(key.segment_idx as usize)
                    .unwrap()
                    .placement();
                fenced.contains(&key.file_id)
                    && !phase_bound.contains(&key.file_id)
                    && !wait_bound.contains(&key.file_id)
                    && !cjs.contains(key.file_id)
                    && !effects.contains(key)
                    && !placement.instantiable()
                    && (!placement.declarations().is_empty() || placement.default_expression())
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
            let facts = &module.parsed.facts();
            let placement = facts
                .emission()
                .segments()
                .get(key.segment_idx as usize)
                .unwrap()
                .placement();
            let definitions = placement
                .declarations()
                .iter()
                .map(|id| {
                    facts
                        .bindings()
                        .iter()
                        .find(|b| b.id() == id)
                        .ok_or_eyre("missing placement binding")
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            if !definitions.is_empty() || placement.default_expression() {
                declarations.insert(key);
            }
            if cjs.contains(key.file_id)
                || wait_bound.contains(&key.file_id)
                || effects.contains(&key)
                || (phase_bound.contains(&key.file_id) && !placement.instantiable())
                || (fenced.contains(&key.file_id)
                    && !placement.instantiable()
                    && !prefixes.contains(&key))
                || !placement.forward_references().is_empty()
            {
                pinned.insert(key);
            }
            for binding in &definitions {
                for declaration in binding.declarations() {
                    let index = module
                        .parsed
                        .segments()
                        .partition_point(|s| s.span.end <= declaration.start);
                    let other = SegmentKey::new(key.file_id, index as u32);
                    if let Some(&other) = indices.get(&other) {
                        constraints[indices[&key]].insert(other);
                        constraints[other].insert(indices[&key]);
                    }
                }
            }
            for index in placement.writes() {
                let reference = facts.references().get(index as usize).unwrap();
                let Some(binding) = reference.binding().and_then(|id| {
                    facts
                        .bindings()
                        .iter()
                        .find(|b| b.id() == id && b.top_level() && !b.import())
                }) else {
                    continue;
                };
                let dependency =
                    declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id()))?
                        .unwrap();
                constraints[indices[&key]].insert(indices[&dependency]);
                constraints[indices[&dependency]].insert(indices[&key]);
            }
        }
        // Eager reads share their initializer's phase; deferred reads can cross
        // chunks as live imports. This also keeps snapshots ahead of later writes.
        for &key in &live {
            let module = &modules[&key.file_id];
            for index in module
                .parsed
                .facts()
                .emission()
                .segments()
                .get(key.segment_idx as usize)
                .unwrap()
                .placement()
                .eager_reads()
            {
                let reference = module
                    .parsed
                    .facts()
                    .references()
                    .get(index as usize)
                    .unwrap();
                let Some(binding) = reference.binding().and_then(|id| {
                    module
                        .parsed
                        .facts()
                        .bindings()
                        .iter()
                        .find(|b| b.id() == id && b.top_level())
                }) else {
                    continue;
                };
                if binding.import() {
                    if let Some(linked) = symbols.imports.get(&(key.file_id, binding.id())) {
                        if let ExportIdentity::Binding(file, id) = linked.identity {
                            if modules[&file]
                                .parsed
                                .facts()
                                .references()
                                .iter()
                                .any(|r| r.binding() == Some(id) && r.write())
                            {
                                pinned.insert(key);
                            }
                        }
                    }
                } else {
                    let dependency =
                        declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id()))?
                            .unwrap();
                    constraints[indices[&key]].insert(indices[&dependency]);
                    constraints[indices[&dependency]].insert(indices[&key]);
                }
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
        for seed in plan
            .roots
            .values()
            .filter(|seed| !matches!(seed.id, RootId::AsyncExport { .. }))
        {
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
            let facts = &modules[&file].parsed.facts();
            if facts.references().iter().any(|reference| {
                if !reference.write() || reference.binding() != Some(binding) {
                    return false;
                }
                let index = modules[&file]
                    .parsed
                    .segments()
                    .partition_point(|segment| segment.span.end <= reference.span().start);
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
        let mut selected_owners = BTreeMap::new();
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
            let mask: BTreeSet<_> = component
                .iter()
                .flat_map(|key| {
                    &plan.node_roots[&NodeId::Segment {
                        module: graph.file_path(key.file_id).unwrap().to_owned(),
                        index: key.segment_idx,
                    }]
                })
                .cloned()
                .collect();
            let owner = if mask
                .iter()
                .any(|root| matches!(root, RootId::AsyncExport { .. }))
            {
                *selected_owners
                    .entry(mask)
                    .or_insert(Owner::Bindings(component[0]))
            } else if let Some(owner) = unique {
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
            let definitions = partition_definitions(graph, prepared, &owners)?;
            let mut rejected = BTreeSet::new();
            let names = connections::name_plan(
                graph,
                plan,
                modules,
                prepared,
                &owners,
                seams,
                &definitions,
            )?;
            for (owner, members) in &activation.members {
                let mut values = Values::new(prepared);
                values.names = names[owner].locals.clone();
                values.requests = seams
                    .iter()
                    .map(|(&file, &owner)| (file, request(graph, owner)))
                    .collect();
                for binding in &names[owner].bindings {
                    values.imports.insert(binding.symbol, binding.alias.clone());
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
                let mut placement =
                    Self::scheduled(graph, plan, modules, owners.clone(), &prefixes)?;
                placement.names = names;
                let (safe, initializers) = activation_safe(
                    graph, plan, modules, symbols, seams, surfaces, &placement, &prefixes, None,
                    None, None,
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
                                    let span = module
                                        .parsed
                                        .segments()
                                        .get(key.segment_idx as usize)
                                        .unwrap()
                                        .span;
                                    module.parsed.facts().references().iter().any(|reference| {
                                        contains(span, reference.span())
                                            && reference.binding().is_some()
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
                        "mandatory execution component crosses unchecked native owners: {:?} ({:?}, prefix={}) -> {:?} ({:?})",
                        constraint.before,
                        owners[&constraint.before],
                        prefixes.contains(&constraint.before),
                        constraint.after,
                        owners[&constraint.after],
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
                let facts = &modules[&access.segment.file_id].parsed.facts();
                if facts.emission().execution().is_module_evaluation(
                    facts
                        .emission()
                        .execution()
                        .references()
                        .get(access.reference as usize)
                        .unwrap(),
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
            names: BTreeMap::new(),
            schedules,
            prefixes: prefixes.clone(),
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
            for reference in module.parsed.facts().references() {
                let segment = module
                    .parsed
                    .segments()
                    .partition_point(|s| s.span.end <= reference.span().start);
                let key = SegmentKey::new(file, segment as u32);
                let Some(&owner) = owners.get(&key) else {
                    continue;
                };
                let Some(binding) = reference.binding() else {
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
    connections: Option<(&BTreeMap<PathBuf, Code>, &BTreeMap<RootId, PathBuf>)>,
    cjs: Option<&commonjs::CommonJs>,
    activations: Option<&Activations>,
) -> eyre::Result<(bool, BTreeSet<SegmentKey>)> {
    let owners = &placement.owners;
    let mut native = BTreeMap::new();
    let mut physical: BTreeMap<Owner, Vec<Owner>> =
        owners.values().map(|&owner| (owner, Vec::new())).collect();
    for (&file, module) in modules {
        let mut dependencies = Vec::new();
        for dependency in module
            .parsed
            .facts()
            .dependencies()
            .iter()
            .filter(|dependency| {
                !dependency.type_only()
                    && matches!(
                        dependency.kind(),
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
                .facts()
                .bindings()
                .iter()
                .filter(|binding| binding.top_level())
            {
                if !module.parsed.facts().references().iter().any(|reference| {
                    if reference.binding() != Some(binding.id()) {
                        return false;
                    }
                    let index = module
                        .parsed
                        .segments()
                        .partition_point(|segment| segment.span.end <= reference.span().start);
                    owners.get(&SegmentKey::new(file, index as u32)) == Some(&owner)
                }) {
                    continue;
                }
                let identity = if symbols.definitions.contains_key(&(file, binding.id())) {
                    ExportIdentity::Binding(file, binding.id())
                } else if let Some(linked) = symbols.imports.get(&(file, binding.id())) {
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
    fn completion<K: Clone + Ord>(roots: Vec<K>, edges: &BTreeMap<K, Vec<K>>) -> Vec<K> {
        let mut seen = BTreeSet::new();
        let mut pending: Vec<_> = roots.into_iter().rev().map(|root| (root, false)).collect();
        let mut order = Vec::new();
        while let Some((node, finish)) = pending.pop() {
            if finish {
                order.push(node);
                continue;
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            pending.push((node.clone(), true));
            pending.extend(
                edges
                    .get(&node)
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|target| (target.clone(), false)),
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
        // Opaque factories/facade jobs have their own activation contract.
        // This final check projects the reconstructed native ESM body events.
        if (connections.is_some()
            && (modules[&key.file_id].parsed.facts().commonjs()
                || cjs.is_some_and(|cjs| cjs.owns_activation(key.file_id))))
            || activations.is_some_and(|a| a.initializers.contains_key(&key.file_id))
        {
            continue;
        }
        let recipe = modules[&key.file_id]
            .parsed
            .facts()
            .emission()
            .segments()
            .get(key.segment_idx as usize)
            .unwrap();
        if recipe.connector_only() || recipe.placement().instantiable() {
            continue;
        }
        events.insert(key);
        native_events.entry(key.file_id).or_default().push(key);
        if !effects.contains(&key)
            && (!recipe.placement().declarations().is_empty()
                || recipe.placement().default_expression())
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
    let reconstructed = connections.map(|(files, routes)| {
        let dependencies: BTreeMap<_, Vec<_>> = files.iter().map(|(path, code)| {
            (path.clone(), code.static_dependencies().map(|(target, _)| target.clone()).collect())
        }).collect();
        let schedules: BTreeMap<_, _> = placement.schedules.iter()
            .filter(|(owner, _)| !matches!(owner, Owner::Evaluation(file) if modules[file].parsed.facts().commonjs()))
            .map(|(owner, schedule)| (owner.path(graph), schedule)).collect();
        (dependencies, schedules, routes)
    });
    for (root, seed) in &plan.roots {
        let mut entrances = seed.evaluation_roots.clone();
        entrances.extend(seed.binding_roots.iter().filter_map(|node| match node {
            GraphNode::Module(file) => Some(*file),
            GraphNode::Segment(key) => Some(key.file_id),
            GraphNode::Unlinked { .. } => None,
        }));
        if activations.is_some_and(|a| {
            entrances
                .iter()
                .all(|file| a.initializers.contains_key(file))
        }) {
            continue;
        }
        let groups = if connections.is_some() {
            vec![entrances]
        } else {
            entrances.into_iter().map(|file| vec![file]).collect()
        };
        for entrances in groups {
            let before = projection(
                completion(entrances.clone(), &native)
                    .into_iter()
                    .flat_map(|file| native_events.get(&file).into_iter().flatten().copied())
                    .collect(),
            );
            let after = if let Some((dependencies, schedules, routes)) = &reconstructed {
                let Some(route) = routes.get(root) else {
                    // External async roots have no owned executable entrance.
                    continue;
                };
                projection(
                    completion(vec![route.clone()], dependencies)
                        .into_iter()
                        .flat_map(|path| {
                            schedules
                                .get(&path)
                                .into_iter()
                                .flat_map(|schedule| schedule.iter().copied())
                        })
                        .collect(),
                )
            } else {
                projection(
                    completion(
                        entrances
                            .iter()
                            .map(|file| Owner::Evaluation(seams[file]))
                            .collect(),
                        &physical,
                    )
                    .into_iter()
                    .flat_map(|owner| placement.schedule(owner).iter().copied())
                    .collect(),
                )
            };
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

#[expect(clippy::too_many_arguments)]
pub(super) fn validate_connections(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    seams: &BTreeMap<u32, u32>,
    surfaces: &BTreeSet<u32>,
    placement: &Placement,
    files: &BTreeMap<PathBuf, Code>,
    routes: &BTreeMap<RootId, PathBuf>,
    cjs: &commonjs::CommonJs,
    activations: Option<&Activations>,
) -> eyre::Result<()> {
    for (path, code) in files {
        for (target, _) in code.static_dependencies() {
            eyre::ensure!(
                files.contains_key(target),
                "native connection in {} targets missing artifact {}",
                path.display(),
                target.display()
            );
        }
    }
    for route in routes.values() {
        eyre::ensure!(
            files.contains_key(route),
            "native entrance targets missing artifact {}",
            route.display()
        );
    }
    if modules.keys().any(|&file| has_externals(graph, file)) {
        return Ok(());
    }
    let (safe, _) = activation_safe(
        graph,
        plan,
        modules,
        symbols,
        seams,
        surfaces,
        placement,
        &BTreeSet::new(),
        Some((files, routes)),
        Some(cjs),
        activations,
    )?;
    eyre::ensure!(
        safe,
        "reconstructed native connections change source initialization phases"
    );
    Ok(())
}

pub(super) fn partition_definitions(
    graph: &SourceGraph,
    prepared: &PreparedValues<'_>,
    owners: &BTreeMap<SegmentKey, Owner>,
) -> eyre::Result<BTreeMap<Owner, Vec<crate::symbols::Symbol>>> {
    let mut definitions: BTreeMap<_, Vec<_>> = BTreeMap::new();
    // Partition the canonical order without sorting by graph-local owner IDs.
    for &symbol @ (file, binding) in &prepared.definitions {
        if let Some(owner) = declaration(graph, &ExportIdentity::Binding(file, binding))?
            .and_then(|key| owners.get(&key))
        {
            definitions.entry(*owner).or_default().push(symbol);
        }
    }
    Ok(definitions)
}

fn binding_closure_safe(
    graph: &SourceGraph,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    owners: &BTreeMap<SegmentKey, Owner>,
    root: Owner,
) -> eyre::Result<bool> {
    let mut edges = BTreeMap::<Owner, BTreeSet<Owner>>::new();
    for (&key, &owner) in owners {
        let facts = &modules[&key.file_id].parsed.facts();
        let span = modules[&key.file_id]
            .parsed
            .segments()
            .get(key.segment_idx as usize)
            .unwrap()
            .span;
        for reference in facts
            .references()
            .iter()
            .filter(|r| !r.type_only() && contains(span, r.span()))
        {
            let Some(binding) = reference.binding() else {
                continue;
            };
            let identity = symbols
                .imports
                .get(&(key.file_id, binding))
                .map(|s| s.identity.clone())
                .unwrap_or(ExportIdentity::Binding(key.file_id, binding));
            let Some(target) = declaration(graph, &identity)? else {
                if matches!(
                    identity,
                    ExportIdentity::Namespace(_) | ExportIdentity::CommonJs(..)
                ) {
                    edges
                        .entry(owner)
                        .or_default()
                        .insert(Owner::Evaluation(key.file_id));
                }
                continue;
            };
            if let Some(&target) = owners.get(&target).filter(|&&target| target != owner) {
                edges.entry(owner).or_default().insert(target);
            }
        }
    }
    let mut pending = vec![(root, false)];
    let mut entered = BTreeSet::new();
    let mut finished = BTreeSet::new();
    while let Some((owner, finish)) = pending.pop() {
        if finish {
            finished.insert(owner);
        } else if !finished.contains(&owner) {
            if matches!(owner, Owner::Evaluation(_)) || !entered.insert(owner) {
                return Ok(false);
            }
            pending.push((owner, true));
            pending.extend(edges.get(&owner).into_iter().flatten().map(|&o| (o, false)));
        }
    }
    Ok(true)
}

pub(super) fn declaration(
    graph: &SourceGraph,
    identity: &ExportIdentity,
) -> eyre::Result<Option<SegmentKey>> {
    Ok(match identity {
        ExportIdentity::Binding(file, binding) => {
            let facts = graph.file_facts(*file).ok_or_eyre("missing symbol facts")?;
            let binding = facts
                .bindings()
                .iter()
                .find(|b| b.id() == *binding)
                .ok_or_eyre("missing symbol binding")?;
            let span = binding
                .declarations()
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
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    owners: &BTreeMap<SegmentKey, Owner>,
    owner: Owner,
    definitions: &[crate::symbols::Symbol],
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
                .facts()
                .emission()
                .free_references()
                .iter()
                .map(|index| {
                    modules[file]
                        .parsed
                        .facts()
                        .references()
                        .get(index as usize)
                        .unwrap()
                        .name()
                        .to_owned()
                })
        })
        .collect();
    let local: HashSet<_> = definitions.iter().copied().collect();
    for &file in &files {
        let module = &modules[&file];
        for reference in module.parsed.facts().references() {
            let Some(binding) = reference.binding() else {
                continue;
            };
            let symbol = (file, binding);
            if local.contains(&symbol) {
                continue;
            }
            let Some(definition) = symbols.definitions.get(&symbol) else {
                continue;
            };
            let index = module
                .parsed
                .segments()
                .partition_point(|segment| segment.span.end <= reference.span().start);
            if owners.get(&SegmentKey::new(file, index as u32)) == Some(&owner) {
                occupied.insert(definition.name.clone());
            }
        }
    }
    let mut names = HashMap::new();
    for &symbol @ (file, binding) in definitions {
        let original = modules[&file]
            .parsed
            .facts()
            .bindings()
            .iter()
            .find(|b| b.id() == binding)
            .ok_or_eyre("missing relocated symbol binding")?;
        let exported = symbols.exports[&file]
            .values()
            .any(|export| export.identity == ExportIdentity::Binding(file, binding));
        let captured = exported
            && files.iter().any(|&other| {
                other != file
                    && modules[&other]
                        .parsed
                        .facts()
                        .emission()
                        .nested_bindings()
                        .iter()
                        .any(|id| {
                            modules[&other]
                                .parsed
                                .facts()
                                .bindings()
                                .iter()
                                .any(|b| b.id() == id && b.name() == original.name())
                        })
            });
        let name = if !captured && occupied.insert(original.name().to_owned()) {
            original.name().to_owned()
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
    modules: &BTreeMap<u32, Module<'_>>,
    prepared: &PreparedValues<'_>,
    placement: &Placement,
    connections: &connections::Connections,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
    cjs: &commonjs::CommonJs,
    activations: Option<&Activations>,
    plan: &ChunkPlan,
) -> eyre::Result<Code> {
    let live: BTreeSet<_> = placement
        .owners
        .iter()
        .filter(|(_, &o)| o == root)
        .map(|(&k, _)| k)
        .collect();
    let mut values = Values::new(prepared);
    values.names = connections
        .names
        .get(&root)
        .cloned()
        .ok_or_eyre("missing planned owner names")?;
    values.requests = connections
        .namespaces
        .iter()
        .map(|(&f, route)| (f, Code::request(route.clone(), RequestKind::DynamicImport)))
        .collect();
    for (site, route) in &connections.async_facades {
        if selected_activation(graph, plan, site) {
            values.dynamic_sites.insert(
                (
                    graph.file_id(&site.module).unwrap(),
                    ast_segmenter::owned::DependencyId(site.offset),
                ),
                Code::request(route.clone(), RequestKind::DynamicImport),
            );
        }
    }
    for boundary in plan.generated_boundaries.values() {
        let GraphNode::Segment(consumer) = graph.edges()[boundary.source_edge].from else {
            unreachable!()
        };
        if !live.contains(&consumer) {
            continue;
        }
        let facts = &modules[&boundary.target.file_id].parsed.facts();
        let binding = facts
            .emission()
            .segments()
            .get(boundary.target.segment_idx as usize)
            .unwrap()
            .placement()
            .declarations()
            .get(0)
            .unwrap();
        let identity = ExportIdentity::Binding(boundary.target.file_id, binding);
        let owner = placement.owners[&boundary.target];
        let expression = if owner == root {
            Code::from(format!(
                "(await 0, {})",
                values
                    .name(&identity)
                    .ok_or_eyre("missing generated local binding")?
            ))
        } else {
            Code::request(owner.path(graph), RequestKind::DynamicImport).surrounded(
                "(0, (await import(",
                format!("))[{}])", quote(connections.address(&identity)?)),
            )
        };
        values
            .generated
            .insert((consumer.file_id, boundary.reference), expression);
    }
    if let Some(activations) = activations {
        for &target in activations.initializers.keys() {
            values.dynamic.insert(
                target,
                activations.dynamic(graph, target, &root.path(graph)),
            );
        }
    }
    for (&file, module) in modules {
        for dependency in module.parsed.facts().dependencies().iter().filter(|d| {
            !d.type_only()
                && d.kind() == DependencyKind::DynamicImport
                && live.iter().any(|key| {
                    key.file_id == file
                        && contains(
                            module
                                .parsed
                                .segments()
                                .get(key.segment_idx as usize)
                                .unwrap()
                                .span,
                            d.span(),
                        )
                })
        }) {
            if let Some(dispatch) =
                cjs.context_dynamic(graph, file, dependency, &connections.namespaces)?
            {
                values.contexts.insert((file, dependency.id()), dispatch);
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
    if let Some(requests) = connections.evaluation.get(&root) {
        output.push(requests.clone());
    }
    let schedule = placement.schedule(root);
    if let Some(imports) = connections.imports.get(&root) {
        output.push(imports.clone());
    }
    for binding in connections.bindings.get(&root).into_iter().flatten() {
        values.imports.insert(binding.symbol, binding.alias.clone());
    }
    let mut imported = BTreeSet::new();
    let mut occupied: BTreeSet<_> = values
        .names
        .values()
        .chain(values.defaults.values())
        .cloned()
        .collect();
    for module in modules.values() {
        occupied.extend(
            module
                .parsed
                .facts()
                .bindings()
                .iter()
                .map(|b| b.name().to_owned()),
        );
        occupied.extend(
            module
                .parsed
                .facts()
                .references()
                .iter()
                .map(|r| r.name().to_owned()),
        );
    }
    occupied.extend(
        prepared
            .symbols
            .definitions
            .values()
            .map(|d| d.name.clone()),
    );
    let mut bodies = Code::default();
    let mut deferred_bodies: BTreeMap<u32, Code> = BTreeMap::new();
    for &key in schedule {
        let file = key.file_id;
        let initialization = activations.and_then(|a| a.initializers.get(&file));
        let (hoisted, body) = reconstruct(
            graph,
            key,
            &modules[&file],
            &values,
            recipes,
            initialization.is_some(),
        )?
        .ok_or_else(|| {
            eyre::eyre!(
                "relocated lexical group changed compatibility: {:?}, segment {}",
                graph.file_path(file),
                key.segment_idx
            )
        })?;
        output.push(hoisted);
        if initialization.is_some() {
            deferred_bodies.entry(file).or_default().push(body);
        } else {
            bodies.push(body);
            bodies.push_str("\n;\n");
        }
    }
    if let Some(activations) = activations {
        for (&file, initialization) in &activations.initializers {
            if Owner::Evaluation(activations.owners[&file]) != root {
                continue;
            }
            let mut dependencies = Code::default();
            for dependency in modules[&file]
                .parsed
                .facts()
                .dependencies()
                .iter()
                .filter(|d| {
                    !d.type_only()
                        && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
                })
            {
                let target = target_file(graph, file, dependency)?;
                let target_init = &activations.initializers[&target].init;
                if Owner::Evaluation(activations.owners[&target]) != root
                    && imported.insert(target_init.clone())
                {
                    output.push(
                        Code::request(
                            path(graph, activations.owners[&target]),
                            RequestKind::Import,
                        )
                        .surrounded(format!("import {{{target_init}}} from "), ";\n"),
                    );
                }
                dependencies.push_str(&format!("{target_init}();\n"));
            }
            if let Some(body) = deferred_bodies.remove(&file) {
                dependencies.push(body);
            }
            output.push(initialization.guard(dependencies));
            let mut getters = String::new();
            for (index, (name, linked)) in prepared.symbols.exports[&file].iter().enumerate() {
                let key = declaration(graph, &linked.identity)?
                    .ok_or_eyre("nonlexical deferred namespace export")?;
                let owner = placement.owners[&key];
                let value = if owner == root {
                    values
                        .name(&linked.identity)
                        .ok_or_eyre("missing deferred namespace binding")?
                        .to_owned()
                } else {
                    let mut alias = format!("{}_{}", initialization.namespace, index);
                    while !occupied.insert(alias.clone()) {
                        alias.push('$');
                    }
                    output.push(
                        Code::request(owner.path(graph), RequestKind::Import).surrounded(
                            format!(
                                "import {{{} as {alias}}} from ",
                                quote(connections.address(&linked.identity)?)
                            ),
                            ";\n",
                        ),
                    );
                    alias
                };
                getters.push_str(&format!("get {}() {{ return {value}; }},\n", quote(name)));
            }
            output.push_str(&format!(
                "var {} = {{__proto__: null,\n{getters}}};\n",
                initialization.namespace
            ));
            if activations.exports.contains(&file) {
                output.push_str(&format!(
                    "export {{{}, {}}};\n",
                    initialization.init, initialization.namespace
                ));
            }
        }
    }
    output.push(bodies);
    if let Some(exports) = connections.exports.get(&root) {
        output.push(exports.clone());
        output.default_export |= exports.default_export;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn definitions_follow_actual_owners_in_canonical_path_and_binding_order() {
        let mut previous = None;
        for reverse in [false, true] {
            let mut inputs: Vec<_> = [
                ("a.mjs", "const z = 1; const a = 2; const unused = 3;"),
                ("z.mjs", "const z = 4; const a = 5;"),
                ("opaque.cjs", "module.exports = 6;"),
            ]
            .into_iter()
            .map(|(path, source)| {
                let parsed = ast_segmenter::parse_module(
                    &logger::StdioLogger::new(),
                    source,
                    Path::new(path),
                    ast_segmenter::ModuleFormat::Esm,
                )
                .unwrap();
                source_graph::OwnedSourceFileInput::from_parsed(
                    path.into(),
                    parsed,
                    Default::default(),
                )
            })
            .collect();
            if reverse {
                inputs.reverse();
            }
            let graph = SourceGraph::from_owned(inputs);
            let a = graph.file_id(Path::new("a.mjs")).unwrap();
            let z = graph.file_id(Path::new("z.mjs")).unwrap();
            let cjs = graph.file_id(Path::new("opaque.cjs")).unwrap();
            let shared = Owner::Bindings(SegmentKey::new(z, 0));
            let evaluation = Owner::Evaluation(cjs);
            let owners = BTreeMap::from([
                (SegmentKey::new(a, 0), shared),
                (SegmentKey::new(a, 1), evaluation),
                (SegmentKey::new(z, 0), shared),
                (SegmentKey::new(z, 1), shared),
                (SegmentKey::new(cjs, 0), evaluation),
            ]);
            let symbols = Symbols {
                definitions: [a, z]
                    .into_iter()
                    .flat_map(|file| {
                        graph
                            .file_facts(file)
                            .unwrap()
                            .bindings()
                            .iter()
                            .map(move |binding| {
                                (
                                    (file, binding.id()),
                                    crate::symbols::Definition {
                                        name: binding.name().to_owned(),
                                    },
                                )
                            })
                    })
                    .collect(),
                imports: HashMap::new(),
                exports: BTreeMap::new(),
            };
            let prepared = PreparedValues::new(&graph, &BTreeMap::new(), &symbols);
            let definitions = partition_definitions(&graph, &prepared, &owners).unwrap();
            let canonical = |owner| {
                definitions[&owner]
                    .iter()
                    .map(|&(file, binding)| {
                        (
                            graph.file_path(file).unwrap().to_owned(),
                            symbols.definitions[&(file, binding)].name.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let actual = (canonical(shared), canonical(evaluation));
            assert_eq!(
                actual,
                (
                    vec![
                        ("a.mjs".into(), "z".into()),
                        ("z.mjs".into(), "z".into()),
                        ("z.mjs".into(), "a".into()),
                    ],
                    vec![("a.mjs".into(), "a".into())],
                )
            );
            assert_eq!(definitions.values().map(Vec::len).sum::<usize>(), 4);
            if let Some(previous) = previous {
                assert_eq!(actual, previous);
            }
            previous = Some(actual);
        }
    }
}
