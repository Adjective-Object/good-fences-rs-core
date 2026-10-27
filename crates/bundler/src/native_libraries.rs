//! Small native libraries change availability, never logical demand or activation.
use super::*;

#[cfg(test)]
#[path = "native_libraries_tests.rs"]
mod tests;

struct Island {
    owner: u32,
    files: BTreeSet<u32>,
    mask: BTreeSet<RootId>,
    bytes: usize,
}

#[expect(clippy::too_many_arguments)]
fn compatible_group(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    group: &BTreeSet<u32>,
    redirects: &BTreeMap<u32, u32>,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    live: &BTreeSet<SegmentKey>,
    cjs: &commonjs::CommonJs,
) -> eyre::Result<bool> {
    let owner = segments::Owner::Evaluation(*group.first().ok_or_eyre("empty native library")?);
    let owners = live
        .iter()
        .map(|&key| {
            (
                key,
                if group.contains(&key.file_id) {
                    owner
                } else {
                    segments::Owner::Evaluation(redirects[&key.file_id])
                },
            )
        })
        .collect();
    let prepared = PreparedValues::new(graph, modules, symbols);
    let definitions = segments::partition_definitions(graph, &prepared, &owners)?;
    let names = connections::name_plan(
        graph,
        plan,
        modules,
        &prepared,
        &owners,
        redirects,
        &definitions,
    )?;
    let mut values = Values::new(&prepared);
    values.names = names[&owner].locals.clone();
    for binding in &names[&owner].bindings {
        values.imports.insert(binding.symbol, binding.alias.clone());
    }
    for &file in group {
        if cjs.contains(file) || !compatible(graph, file, &modules[&file], &values, live)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[expect(clippy::too_many_arguments)]
pub(super) fn cluster(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    groups: &BTreeMap<u32, BTreeSet<u32>>,
    redirects: &BTreeMap<u32, u32>,
    live: &BTreeSet<SegmentKey>,
    cjs: &commonjs::CommonJs,
    activations: Option<&Activations>,
    target: usize,
) -> eyre::Result<Option<Activations>> {
    if target == 0 || modules.keys().any(|&f| has_externals(graph, f)) {
        return Ok(None);
    }
    let mut seams: BTreeMap<_, _> = groups
        .iter()
        .flat_map(|(&owner, files)| files.iter().map(move |&file| (file, owner)))
        .collect();
    for (&file, &target) in redirects {
        seams.insert(file, seams[&target]);
    }
    let placements = activations.map(|a| a.owners.clone()).unwrap_or_default();
    let selected_roots: BTreeSet<_> = plan
        .async_activations
        .iter()
        .filter(|(site, _)| selected_activation(graph, plan, site))
        .flat_map(|(_, activation)| activation.roots.iter().cloned())
        .collect();
    let mut islands = Vec::new();
    for (&owner, files) in groups {
        // Existing parent moves keep their certified owner, including its static closure.
        if files
            .iter()
            .any(|f| placements.contains_key(f) || placements.values().any(|o| o == f))
        {
            continue;
        }
        let keys: Vec<_> = live.iter().filter(|k| files.contains(&k.file_id)).collect();
        let mask: BTreeSet<_> = keys
            .iter()
            .flat_map(|&&key| {
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
                    .map(|span| (span.end - span.start) as usize)
                    .sum::<usize>()
            })
            .sum();
        if mask.is_empty()
            || bytes >= target
            || !mask.is_disjoint(&selected_roots)
            || mask
                .iter()
                .any(|r| matches!(r, RootId::Entry(_) | RootId::AsyncExport { .. }))
        {
            continue;
        }
        let candidate: BTreeMap<_, _> = files.iter().map(|&f| (f, owner)).collect();
        if certify(
            graph,
            plan,
            modules,
            symbols,
            &seams,
            &placements,
            &candidate,
        )?
        .is_some()
        {
            islands.push(Island {
                owner,
                files: files.clone(),
                mask,
                bytes,
            });
        }
    }
    islands.sort_by_key(|island| {
        live.iter()
            .filter(|k| island.files.contains(&k.file_id))
            .map(|k| (graph.file_path(k.file_id).unwrap(), k.segment_idx))
            .min()
    });
    let inputs: Vec<_> = islands.iter().map(|i| (i.mask.clone(), i.bytes)).collect();
    let selected = select(
        &inputs,
        target,
        placements.clone(),
        |existing, members, candidate| {
            let seed = *members.first().unwrap();
            let files: BTreeSet<_> = members
                .iter()
                .copied()
                .chain([candidate])
                .flat_map(|i| islands[i].files.iter().copied())
                .collect();
            if !compatible_group(graph, plan, &files, redirects, modules, symbols, live, cjs)? {
                return Ok(None);
            }
            let moves = files.iter().map(|&f| (f, islands[seed].owner)).collect();
            Ok(certify(graph, plan, modules, symbols, &seams, existing, &moves)?.map(|a| a.owners))
        },
    )?;
    if selected != placements {
        Activations::checked(graph, plan, modules, symbols, &selected)
    } else {
        Ok(None)
    }
}

/// Inputs are already in canonical source/segment order and individually undersized.
pub(super) fn select<S>(
    islands: &[(BTreeSet<RootId>, usize)],
    target: usize,
    mut state: S,
    mut merge: impl FnMut(&S, &BTreeSet<usize>, usize) -> eyre::Result<Option<S>>,
) -> eyre::Result<S> {
    let mut used = BTreeSet::new();
    for (seed, (initial_mask, initial_bytes)) in islands.iter().enumerate() {
        if !used.insert(seed) {
            continue;
        }
        let mut members = BTreeSet::from([seed]);
        let mut mask = initial_mask.clone();
        let mut bytes = *initial_bytes;
        while bytes < target {
            let mut best = None;
            for (index, (candidate_mask, _)) in islands.iter().enumerate() {
                if used.contains(&index) {
                    continue;
                }
                let intersection = mask.intersection(candidate_mask).count();
                if intersection == 0 {
                    continue;
                }
                let union = mask.union(candidate_mask).count();
                if best.as_ref().is_some_and(|(_, n, d, _)| {
                    (intersection as u128) * (*d as u128) <= (*n as u128) * (union as u128)
                }) {
                    continue;
                }
                let Some(proposal) = merge(&state, &members, index)? else {
                    continue;
                };
                best = Some((index, intersection, union, proposal));
            }
            let Some((index, _, _, proposal)) = best else {
                break;
            };
            used.insert(index);
            members.insert(index);
            mask.extend(islands[index].0.iter().cloned());
            bytes = bytes.saturating_add(islands[index].1);
            state = proposal;
        }
    }
    Ok(state)
}

/// Guard synchronous consumers as well as dependencies, without moving their bytes.
fn certify(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    seams: &BTreeMap<u32, u32>,
    existing: &BTreeMap<u32, u32>,
    moves: &BTreeMap<u32, u32>,
) -> eyre::Result<Option<Activations>> {
    let mut selected: BTreeSet<_> = moves.keys().copied().collect();
    let mut edges = Vec::new();
    for (&file, module) in modules {
        for dependency in module.parsed.facts().dependencies().iter().filter(|d| {
            !d.type_only()
                && matches!(
                    d.kind(),
                    DependencyKind::Import
                        | DependencyKind::ImportMetaEnv
                        | DependencyKind::ReExport
                )
        }) {
            for target in target_files(graph, file, dependency)? {
                edges.push((file, target));
            }
        }
    }
    loop {
        let before = selected.len();
        for &(file, target) in &edges {
            if selected.contains(&file) || selected.contains(&target) {
                selected.insert(file);
                selected.insert(target);
            }
        }
        if selected.len() == before {
            break;
        }
    }
    let mut placements = existing.clone();
    for &file in &selected {
        if modules[&file].parsed.facts().emission().uses_this() {
            return Ok(None);
        }
        placements.entry(file).or_insert(seams[&file]);
    }
    for (&file, &owner) in moves {
        if existing
            .get(&file)
            .is_some_and(|&old| old != seams[&file] && old != owner)
        {
            return Ok(None);
        }
        placements.insert(file, owner);
    }
    let Some(checked) = Activations::checked(graph, plan, modules, symbols, &placements)? else {
        return Ok(None);
    };
    if plan.async_activations.iter().any(|(site, activation)| {
        selected_activation(graph, plan, site)
            && matches!(
                graph.edges()[activation.source_edge].to,
                GraphNode::Module(target) if checked.initializers.contains_key(&target)
            )
    }) {
        return Ok(None);
    }
    for (&file, module) in modules {
        for dependency in module.parsed.facts().dependencies().iter().filter(|d| {
            !d.type_only()
                && d.kind() == DependencyKind::DynamicImport
                && plan.roots.contains_key(&RootId::Async(AsyncSiteId {
                    module: graph.file_path(file).unwrap().to_owned(),
                    offset: d.id().0,
                    candidate: None,
                }))
        }) {
            for target in target_files(graph, file, dependency)? {
                if checked.initializers.contains_key(&target)
                    && !super::super::dynamic_namespace_safe(
                        modules, symbols, file, dependency, target,
                    )
                {
                    return Ok(None);
                }
            }
        }
    }
    Ok(Some(checked))
}
