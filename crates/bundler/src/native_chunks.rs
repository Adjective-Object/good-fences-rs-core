//! Native source jobs retain phases; pure segments have independent physical owners.
use super::*;
use crate::assembly::{Code, RequestKind};
use ast_segmenter::owned::ImportedName;

#[path = "native_cjs.rs"]
mod commonjs;

#[path = "native_segments.rs"]
mod segments;

#[path = "native_connections.rs"]
mod connections;
#[path = "native_libraries.rs"]
mod libraries;

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
) -> eyre::Result<Artifacts<Code>> {
    emit_inner(graph, plan, modules, manifest, prepared, browser, require_defaults, None, 0)
}

pub(super) fn emit_activated(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
    placements: &BTreeMap<u32, u32>,
    min_chunk_bytes: usize,
) -> eyre::Result<Artifacts<Code>> {
    let symbols = prepared.symbols;
    if placements.is_empty() {
        return emit_inner(
            graph,
            plan,
            modules,
            manifest,
            prepared, browser, require_defaults,
            None,
            min_chunk_bytes,
        );
    }
    let Some(activations) = Activations::checked(graph, plan, modules, symbols, placements)? else {
        return super::ordinary(graph, plan, modules, manifest, prepared, browser, require_defaults);
    };
    emit_inner(
        graph,
        plan,
        modules,
        manifest,
        prepared, browser, require_defaults,
        Some(&activations),
        min_chunk_bytes,
    )
}

fn emit_inner(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
    activations: Option<&Activations>,
    min_chunk_bytes: usize,
) -> eyre::Result<Artifacts<Code>> {
    let symbols = prepared.symbols;
    let mut entry_owners = BTreeMap::new();
    let mut async_targets = BTreeMap::new();
    for (root, seed) in &plan.roots {
        match root {
            RootId::Entry(entry) => {
                let mut owners = Vec::new();
                for &file in &seed.evaluation_roots {
                    if !owners.contains(&file) {
                        owners.push(file);
                    }
                }
                for node in &seed.binding_roots {
                    let file = match node {
                        GraphNode::Module(file) => *file,
                        GraphNode::Segment(key) => key.file_id,
                        GraphNode::Unlinked { .. } => eyre::bail!("unlinked native entry root"),
                    };
                    if !owners.contains(&file) {
                        owners.push(file);
                    }
                }
                entry_owners.insert(entry.clone(), owners);
            }
            RootId::Async(site) => {
                if plan.generated_boundaries.contains_key(site) {
                    continue;
                }
                if matches!(manifest.async_facades[site].target, NodeId::External { .. }) {
                    continue;
                }
                let NodeId::Module(target) = &manifest.async_facades[site].target else {
                    eyre::bail!("nonmodule async target")
                };
                async_targets.insert(site.clone(), graph.file_id(target).unwrap());
            }
        }
    }
    let native_seams = modules.keys().any(|&file| has_externals(graph, file));
    let async_modules = super::super::async_modules(graph, modules)?;
    let mut cjs = commonjs::CommonJs::new(graph, plan, modules, browser, require_defaults)?;
    let asynchronous: BTreeSet<_> = async_modules
        .iter()
        .map(|p| graph.file_id(p).unwrap())
        .collect();
    let mut edges = BTreeMap::new();
    let mut connectors = BTreeSet::new();
    let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
    for (&file, module) in modules {
        if cjs.contains(file) {
            edges.insert(file, Vec::new());
            continue;
        }
        if module
            .parsed
            .facts
            .emission
            .segments
            .iter()
            .enumerate()
            .filter(|(index, _)| live.contains(&SegmentKey::new(file, *index as u32)))
            .all(|(_, recipe)| recipe.connector_only)
        {
            connectors.insert(file);
        }
        edges.insert(
            file,
            module
                .parsed
                .facts
                .dependencies
                .iter()
                .filter(|d| {
                    !d.type_only
                        && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
                })
                .filter(|d| {
                    !matches!(
                        graph.dependency_target(file, d.id),
                        Some(DependencyTarget::External(_))
                    )
                })
                .map(|d| target_file(graph, file, d))
                .collect::<eyre::Result<Vec<_>>>()?,
        );
    }
    let mut boundaries: BTreeSet<_> = entry_owners.values().flatten().copied().collect();
    boundaries.extend(async_targets.values().copied());
    if let Some(activations) = activations {
        boundaries.extend(activations.owners.keys().copied());
        boundaries.extend(activations.owners.values().copied());
    }
    // Keep source wait bounds and sibling interleaving in native module jobs.
    boundaries.extend(&asynchronous);
    boundaries.extend(modules.keys().filter(|&&f| cjs.contains(f)).copied());
    if native_seams {
        boundaries.extend(modules.keys().copied());
    }
    for (&file, module) in modules {
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .filter(|d| !d.type_only && d.kind == DependencyKind::Require)
        {
            boundaries.extend(target_files(graph, file, dependency)?);
        }
    }
    for linked in symbols
        .imports
        .values()
        .chain(symbols.exports.values().flat_map(|e| e.values()))
    {
        if let ExportIdentity::Namespace(file) = linked.identity {
            boundaries.insert(file);
        }
    }
    // Cyclic evaluation remains entrance-dependent, even for a complete SCC.
    let files: Vec<_> = modules.keys().copied().collect();
    let indices: BTreeMap<_, _> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let dependencies: Vec<_> = files
        .iter()
        .map(|f| edges[f].iter().map(|t| indices[t]).collect())
        .collect();
    let cycle_groups: Vec<BTreeSet<_>> = crate::effect_dependencies::components(&dependencies)
        .into_iter()
        .filter(|c| c.len() > 1 || dependencies[c[0]].contains(&c[0]))
        .map(|component| component.into_iter().map(|i| files[i]).collect())
        .collect();
    let cycles: BTreeSet<_> = cycle_groups.iter().flatten().copied().collect();
    let mut redirects: BTreeMap<_, _> = modules.keys().map(|&f| (f, f)).collect();
    for &file in &files {
        let targets: BTreeSet<_> = edges[&file].iter().copied().collect();
        let Some(&target) = targets.first().filter(|_| targets.len() == 1) else {
            continue;
        };
        // Redirect only identical surfaces: a later export-star observes extras.
        if connectors.contains(&file)
            && !cjs.contains(target)
            && !boundaries.contains(&file)
            && !cycles.contains(&file)
            && symbols.exports[&file].len() == symbols.exports[&target].len()
            && symbols.exports[&file].iter().all(|(name, linked)| {
                symbols.exports[&target]
                    .get(name)
                    .is_some_and(|e| e.identity == linked.identity)
            })
        {
            redirects.insert(file, target);
        }
    }
    for &file in &files {
        let mut target = redirects[&file];
        while target != redirects[&target] {
            target = redirects[&target];
        }
        redirects.insert(file, target);
    }
    for targets in edges.values_mut() {
        for target in targets {
            *target = redirects[target];
        }
    }
    let recipes = plan
        .retention
        .effects
        .iter()
        .filter(|e| !e.summary.rewrites.is_empty())
        .map(|e| (e.key, e.summary.rewrites.as_slice()))
        .collect();
    let mut owners = redirects.clone();
    let mut jobs: BTreeSet<_> = modules
        .keys()
        .filter(|&&f| redirects[&f] == f)
        .copied()
        .collect();
    let phase_bound = cycles.union(&asynchronous).copied().collect();
    // Changing static requests inside an async closure can change host pending-
    // dependency accounting even when a scalar completion projection agrees.
    let mut wait_bound = BTreeSet::new();
    let mut pending: Vec<_> = asynchronous.iter().copied().collect();
    while let Some(file) = pending.pop() {
        if wait_bound.insert(file) {
            pending.extend(edges[&file].iter().copied());
        }
    }
    let groups = jobs
        .iter()
        .map(|&file| (file, BTreeSet::from([file])))
        .collect();
    let clustered = libraries::cluster(
        graph,
        plan,
        modules,
        symbols,
        &groups,
        &redirects,
        &live,
        &cjs,
        activations,
        min_chunk_bytes,
    )?;
    let activations = clustered.as_ref().or(activations);
    if let Some(activations) = activations {
        for (&file, &owner) in &activations.owners {
            let file = owners[&file];
            let owner = owners[&owner];
            if file == owner {
                continue;
            }
            if cjs.contains(owner)
                || asynchronous.contains(&owner)
                || cycles.contains(&owner)
            {
                return super::ordinary(graph, plan, modules, manifest, prepared, browser, require_defaults);
            }
            jobs.remove(&file);
            for assigned in owners.values_mut() {
                if *assigned == file {
                    *assigned = owner;
                }
            }
        }
    }
    let mut placement = segments::Placement::new(
        graph,
        plan,
        modules,
        prepared,
        &redirects,
        &phase_bound,
        &wait_bound,
        &boundaries,
        &cjs,
    )?;
    placement.extract_generated(graph, plan)?;
    if let Some(activations) = activations {
        for (&key, owner) in &mut placement.owners {
            if activations.initializers.contains_key(&key.file_id) {
                *owner = segments::Owner::Evaluation(owners[&key.file_id]);
            } else if let segments::Owner::Evaluation(file) = owner {
                *file = owners[file];
            }
        }
        placement.reschedule(graph, plan, modules, prepared, &owners)?;
        for (&owner, names) in &placement.names {
            let mut values = Values::new(prepared);
            values.names = names.locals.clone();
            for binding in &names.bindings {
                values.imports.insert(binding.symbol, binding.alias.clone());
            }
            let retained = placement.schedule(owner).iter().copied().collect();
            for file in placement
                .schedule(owner)
                .iter()
                .map(|key| key.file_id)
                .collect::<BTreeSet<_>>()
            {
                if !compatible(graph, file, &modules[&file], &values, &retained)? {
                    return super::ordinary(graph, plan, modules, manifest, prepared, browser, require_defaults);
                }
            }
        }
    }
    placement.cluster(graph, plan, modules, prepared, &owners, min_chunk_bytes)?;
    placement.hoist_generated(graph, plan, modules, min_chunk_bytes)?;
    placement.reschedule(graph, plan, modules, prepared, &owners)?;
    let resolved_activations = activations
        .map(|activations| {
            let mut resolved = activations.clone();
            resolved.exports.clear();
            for (&file, owner) in &mut resolved.owners {
                *owner = owners[&file];
            }
            for (&file, module) in modules {
                for dependency in module
                    .parsed
                    .facts
                    .dependencies
                    .iter()
                    .filter(|d| !d.type_only)
                {
                    let consumer = if dependency.kind == DependencyKind::DynamicImport {
                        let index = module
                            .parsed
                            .segments
                            .partition_point(|s| s.span.end <= dependency.span.start);
                        let Some(owner) =
                            placement.owners.get(&SegmentKey::new(file, index as u32))
                        else {
                            continue;
                        };
                        *owner
                    } else if resolved.initializers.contains_key(&file) {
                        segments::Owner::Evaluation(owners[&file])
                    } else {
                        continue;
                    };
                    for target in target_files(graph, file, dependency)? {
                        if resolved.initializers.contains_key(&target)
                            && consumer != segments::Owner::Evaluation(owners[&target])
                        {
                            resolved.exports.insert(target);
                        }
                    }
                }
            }
            Ok::<_, eyre::Report>(resolved)
        })
        .transpose()?;
    let activations = resolved_activations.as_ref();
    if let Some(activations) = activations {
        if activations.exports.iter().any(|file| {
            owners[file] != *file
                && entry_owners
                    .values()
                    .flatten()
                    .any(|owner| *owner == owners[file])
        }) {
            return super::ordinary(graph, plan, modules, manifest, prepared, browser, require_defaults);
        }
    }
    let mut changed = BTreeSet::new();
    for boundary in plan.generated_boundaries.values() {
        let GraphNode::Segment(consumer) = graph.edges()[boundary.source_edge].from else {
            unreachable!()
        };
        changed.insert(owners[&consumer.file_id]);
    }
    if activations.is_some() {
        changed.extend(jobs.iter().copied());
    }
    changed.extend(
        jobs.iter()
            .filter(|file| connectors.contains(file) && !boundaries.contains(file))
            .copied(),
    );
    for (&key, &owner) in &placement.owners {
        if owner != segments::Owner::Evaluation(owners[&key.file_id])
            || matches!(owner, segments::Owner::Evaluation(file) if file != key.file_id)
        {
            changed.insert(owners[&key.file_id]);
            if let segments::Owner::Evaluation(file) = owner {
                changed.insert(file);
            }
        }
    }
    let mut consumers: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (&file, dependencies) in &edges {
        for target in dependencies {
            consumers
                .entry(owners[target])
                .or_default()
                .insert(owners[&file]);
        }
    }
    let mut pending: Vec<_> = changed.iter().copied().collect();
    while let Some(file) = pending.pop() {
        for &consumer in consumers.get(&file).into_iter().flatten() {
            if changed.insert(consumer) {
                pending.push(consumer);
            }
        }
    }
    // Opaque external linking and unchanged async request accounting keep their
    // native clauses. Other ESM jobs share canonical post-placement connections.
    if !native_seams {
        changed.extend(
            jobs.iter()
                .filter(|&&file| !cjs.contains(file) && !wait_bound.contains(&file))
                .copied(),
        );
    }
    let mut surfaces = boundaries.clone();
    surfaces.extend(jobs.iter().filter(|f| !changed.contains(f)).copied());
    if let Some(activations) = activations {
        for (&file, &owner) in &activations.owners {
            if file != owner && activations.exports.contains(&file) {
                surfaces.remove(&owner);
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut namespaces = BTreeMap::new();
    let entry_files = entry_owners.values().flatten().copied().collect();
    let connections = connections::Connections::new(
        graph, modules, prepared, &owners, &placement, &surfaces, &cjs, activations, plan,
    )?;
    cjs.emit(graph, modules, &connections.namespaces, &entry_files, plan, prepared)?;
    for &root in &jobs {
        if cjs.contains(root) {
            continue;
        }
        let artifact_path = path(graph, root);
        let output = if changed.contains(&root) {
            segments::emit(
                graph,
                segments::Owner::Evaluation(root),
                modules,
                prepared,
                &placement,
                &connections,
                &recipes,
                &cjs,
                activations,
                plan,
            )?
        } else {
            let module = &modules[&root];
            let facts = &module.parsed.facts;
            let edits = facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only)
                .filter(|dependency| {
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|segment| segment.span.end <= dependency.span.start);
                    live.contains(&SegmentKey::new(root, index as u32))
                })
                .filter(|d| d.pattern.is_none())
                .map(|dependency| {
                    let request = connections
                        .native_requests
                        .get(&(root, dependency.id))
                        .ok_or_eyre("missing planned native request")?;
                    Ok((dependency, request))
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            let mut imports = connections
                .evaluation
                .get(&segments::Owner::Evaluation(root))
                .cloned()
                .unwrap_or_default();
            for &key in placement.schedule(segments::Owner::Evaluation(root)) {
                eyre::ensure!(
                    key.file_id == root,
                    "relocated segment in unchanged native job"
                );
                let index = key.segment_idx as usize;
                let recipe = &facts.emission.segments[index];
                let mut renderer = crate::emit::CachedRenderer::new(
                    module.source,
                    facts,
                    recipe,
                    recipes.get(&key).copied().unwrap_or_default(),
                );
                renderer.relocate_urls(module.relocations)?;
                for (dependency, request) in &edits {
                    renderer.dependency(dependency, request.code.clone(), request.whole);
                }
                for dependency in facts.dependencies.iter().filter(|d| {
                    d.pattern.is_some() && contains(module.parsed.segments[index].span, d.span)
                }) {
                    if let Some(dispatch) =
                        cjs.context_dynamic(graph, root, dependency, &connections.namespaces)?
                    {
                        renderer.context(dependency, dispatch);
                    }
                }
                imports.push(renderer.materialize()?);
                imports.push_str("\n;\n");
            }
            imports.default_export |= facts.exports.iter().any(|export| {
                !export.type_only
                    && export.name.as_deref() == Some("default")
                    && module
                        .parsed
                        .segments
                        .iter()
                        .enumerate()
                        .any(|(index, segment)| {
                            live.contains(&SegmentKey::new(root, index as u32))
                                && contains(segment.span, export.span)
                        })
            });
            imports
        };
        if surfaces.contains(&root)
            && !activations.is_some_and(|a| a.initializers.contains_key(&root))
        {
            namespaces.insert(
                graph.file_path(root).unwrap().to_owned(),
                connections.namespaces[&root].clone(),
            );
        }
        files.insert(artifact_path, output);
    }
    for owner in placement.owners.values().copied().collect::<BTreeSet<_>>() {
        if matches!(owner, segments::Owner::Bindings(_)) {
            let output = segments::emit(
                graph,
                owner,
                modules,
                prepared,
                &placement,
                &connections,
                &recipes,
                &cjs,
                activations,
                plan,
            )?;
            let path = owner.path(graph);
            files.insert(path, output);
        }
    }
    for (&file, path) in &cjs.entries {
        namespaces.insert(graph.file_path(file).unwrap().to_owned(), path.clone());
    }
    for (path, output) in &cjs.files {
        files.insert(path.clone(), output.clone());
    }
    files.extend(
        connections
            .projections
            .iter()
            .map(|(path, code)| (path.clone(), code.clone())),
    );
    let mut members = BTreeMap::new();
    let mut chunk_artifacts = BTreeMap::new();
    for (chunk, nodes) in &plan.chunks {
        let mut paths = BTreeSet::new();
        for node in nodes {
            if matches!(node, NodeId::External { .. }) {
                continue;
            }
            if let Some(owner) = cjs.members.get(node) {
                paths.insert(owner.clone());
                members.insert(node.clone(), owner.clone());
                continue;
            }
            let module = match node {
                NodeId::Module(p) | NodeId::Segment { module: p, .. } => p,
                _ => eyre::bail!("non-ESM node in native plan"),
            };
            let file = graph.file_id(module).unwrap();
            let owner = match node {
                NodeId::Segment { index, .. } => {
                    placement.owners[&SegmentKey::new(file, *index)].path(graph)
                }
                _ => path(graph, owners[&file]),
            };
            paths.insert(owner.clone());
            members.insert(node.clone(), owner);
        }
        chunk_artifacts.insert(chunk.clone(), paths);
    }
    let mut entry_paths = BTreeMap::new();
    for (entry, roots) in entry_owners {
        let root = &plan.roots[&RootId::Entry(entry.clone())];
        let module_path = |file| {
            cjs.entries
                .get(&file)
                .cloned()
                .unwrap_or_else(|| connections.namespaces[&file].clone())
        };
        if let [file] = roots.as_slice() {
            // Multi-owner roots never invent a combined public namespace.
            if cjs.contains(*file)
                || !root.binding_roots.is_empty()
                || (symbols.exports[file].is_empty()
                    && plan.node_roots[&NodeId::Module(graph.file_path(*file).unwrap().to_owned())]
                        .len()
                        == 1)
            {
                entry_paths.insert(entry, module_path(*file));
                continue;
            }
        }
        let entry_path = PathBuf::from(format!("native/entry-{}.mjs", digest(&entry)));
        let mut source = Code::default();
        for file in roots {
            source.push(Code::evaluation(module_path(file)));
        }
        files.insert(entry_path.clone(), source);
        entry_paths.insert(entry, entry_path);
    }
    let mut root_routes: BTreeMap<_, _> = entry_paths
        .iter()
        .map(|(entry, path)| (RootId::Entry(entry.clone()), path.clone()))
        .collect();
    root_routes.extend(async_targets.iter().map(|(site, file)| {
        (
            RootId::Async(site.clone()),
            cjs.namespaces
                .get(graph.file_path(*file).unwrap())
                .cloned()
                .unwrap_or_else(|| connections.namespaces[file].clone()),
        )
    }));
    let empty_jobs: BTreeSet<_> = jobs
        .iter()
        .filter(|file| {
            !boundaries.contains(file)
                && !wait_bound.contains(file)
                && !cjs.contains(**file)
                && placement
                    .owners
                    .iter()
                    .filter(|(_, owner)| **owner == segments::Owner::Evaluation(**file))
                    .all(|(key, _)| {
                        modules[&key.file_id].parsed.facts.emission.segments
                            [key.segment_idx as usize]
                            .connector_only
                    })
        })
        .map(|&file| path(graph, file))
        .collect();
    Code::prune(&mut files, &empty_jobs, &mut members)?;
    namespaces.retain(|_, path| files.contains_key(path));
    for (chunk, nodes) in &plan.chunks {
        let mut paths: BTreeSet<_> = nodes
            .iter()
            .filter_map(|node| members.get(node).cloned())
            .collect();
        for node in nodes {
            if let NodeId::Module(module) = node {
                if let Some(route) = namespaces.get(module) {
                    if connections.projections.contains_key(route) {
                        paths.insert(route.clone());
                    }
                }
            }
        }
        chunk_artifacts.insert(chunk.clone(), paths);
    }
    cjs.add_runtime(&mut files, &mut chunk_artifacts)?;
    segments::validate_connections(
        graph,
        plan,
        modules,
        symbols,
        &owners,
        &surfaces,
        &placement,
        &files,
        &root_routes,
        &cjs,
        activations,
    )?;
    Ok(Artifacts {
        resources: BTreeMap::new(),
        stylesheets: BTreeMap::new(),
        backend: Backend::Native,
        files,
        entries: entry_paths,
        modules: namespaces,
        members,
        chunk_artifacts,
        manifest: manifest.clone(),
        cjs_proxies: cjs.proxies,
        cjs_namespaces: cjs.namespaces.clone(),
        async_facades: async_targets
            .into_iter()
            .map(|(site, file)| {
                (
                    site,
                    cjs.namespaces
                        .get(graph.file_path(file).unwrap())
                        .cloned()
                        .unwrap_or_else(|| connections.namespaces[&file].clone()),
                )
            })
            .chain(plan.generated_boundaries.iter().map(|(site, boundary)| {
                (site.clone(), placement.owners[&boundary.target].path(graph))
            }))
            .collect(),
        async_modules,
    })
}

pub(super) fn path(graph: &SourceGraph, file: u32) -> PathBuf {
    PathBuf::from(format!(
        "native/{}.mjs",
        digest(graph.file_path(file).unwrap().to_str().unwrap())
    ))
}

fn request(graph: &SourceGraph, file: u32) -> Code {
    Code::request(path(graph, file), RequestKind::DynamicImport)
}
