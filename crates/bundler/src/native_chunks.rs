//! Native source jobs retain phases; pure segments have independent physical owners.
use super::*;
use crate::assembly::{Code, RequestKind};
use ast_segmenter::owned::ImportedName;

#[path = "native_cjs.rs"]
mod commonjs;

#[path = "native_segments.rs"]
mod segments;

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts<Code>> {
    let _span = tracing::info_span!("emit.chunks").entered();
    let planning_span = tracing::info_span!("emit.planning").entered();
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
    let mut cjs = commonjs::CommonJs::new(graph, plan, modules)?;
    let native_seams = modules.keys().any(|&file| has_externals(graph, file));
    let async_modules = super::super::async_modules(graph, modules)?;
    let asynchronous: BTreeSet<_> = async_modules
        .iter()
        .map(|p| graph.file_id(p).unwrap())
        .collect();
    let mut edges = BTreeMap::new();
    let mut connectors = BTreeSet::new();
    let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
    for (&file, module) in modules {
        if module.parsed.facts().commonjs() {
            edges.insert(file, Vec::new());
            continue;
        }
        if module
            .parsed
            .facts()
            .emission()
            .segments()
            .iter()
            .enumerate()
            .filter(|(index, _)| live.contains(&SegmentKey::new(file, *index as u32)))
            .all(|(_, recipe)| recipe.connector_only())
        {
            connectors.insert(file);
        }
        edges.insert(
            file,
            module
                .parsed
                .facts()
                .dependencies()
                .iter()
                .filter(|d| {
                    !d.type_only()
                        && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
                })
                .filter(|d| {
                    !matches!(
                        graph.dependency_target(file, d.id()),
                        Some(DependencyTarget::External(_))
                    )
                })
                .map(|d| target_file(graph, file, d))
                .collect::<eyre::Result<Vec<_>>>()?,
        );
    }
    let mut boundaries: BTreeSet<_> = entry_owners.values().flatten().copied().collect();
    boundaries.extend(async_targets.values().copied());
    // Keep source wait bounds and sibling interleaving in native module jobs.
    boundaries.extend(&asynchronous);
    boundaries.extend(modules.keys().filter(|&&f| cjs.contains(f)).copied());
    if native_seams {
        boundaries.extend(modules.keys().copied());
    }
    for (&file, module) in modules {
        for dependency in module
            .parsed
            .facts()
            .dependencies()
            .iter()
            .filter(|d| !d.type_only() && d.kind() == DependencyKind::Require)
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
    let owners = redirects.clone();
    let jobs: BTreeSet<_> = modules
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
    let placement = tracing::info_span!("emit.placement").in_scope(|| {
        segments::Placement::new(
            graph,
            plan,
            modules,
            prepared,
            &owners,
            &phase_bound,
            &wait_bound,
            &boundaries,
            &cjs,
        )
    })?;
    let mut changed = BTreeSet::new();
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
    let mut surfaces = boundaries.clone();
    surfaces.extend(jobs.iter().filter(|f| !changed.contains(f)).copied());
    let mut files = BTreeMap::new();
    let mut namespaces = BTreeMap::new();
    let entry_files = entry_owners.values().flatten().copied().collect();
    let routing = tracing::info_span!("emit.routing")
        .in_scope(|| segments::Routing::new(graph, prepared, &placement, &surfaces))?;
    drop(planning_span);
    cjs.emit(graph, modules, &owners, &entry_files, execution)?;
    let mut owner_jobs: Vec<_> = jobs
        .iter()
        .filter(|&&root| !cjs.contains(root))
        .map(|&root| segments::Owner::Evaluation(root))
        .chain(
            placement
                .owners
                .values()
                .copied()
                .filter(|owner| matches!(owner, segments::Owner::Bindings(_))),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    owner_jobs.sort_by_key(|owner| match owner {
        segments::Owner::Evaluation(file) => (graph.file_path(*file), 0, 0),
        segments::Owner::Bindings(key) => (graph.file_path(key.file_id), 1, key.segment_idx),
    });
    let outputs = execution.map(&owner_jobs, |&owner| -> eyre::Result<_> {
        let (file_id, segment_id) = match owner {
            segments::Owner::Evaluation(file) => (file, None),
            segments::Owner::Bindings(key) => (key.file_id, Some(key.segment_idx)),
        };
        let _span = tracing::info_span!("emit.esm", file_id, segment_id).entered();
        let segments::Owner::Evaluation(root) = owner else {
            let output = segments::emit(
                graph, owner, &owners, &redirects, modules, prepared, &placement, &surfaces,
                &recipes, &cjs, &routing,
            )?;
            return Ok((owner.path(graph), output, None));
        };
        let artifact_path = path(graph, root);
        let output = if changed.contains(&root) {
            segments::emit(
                graph,
                segments::Owner::Evaluation(root),
                &owners,
                &redirects,
                modules,
                prepared,
                &placement,
                &surfaces,
                &recipes,
                &cjs,
                &routing,
            )?
        } else {
            let module = &modules[&root];
            let facts = &module.parsed.facts();
            let edits = facts
                .dependencies()
                .iter()
                .filter(|d| !d.type_only())
                .filter(|dependency| {
                    let index = module
                        .parsed
                        .segments()
                        .partition_point(|segment| segment.span.end <= dependency.span().start);
                    live.contains(&SegmentKey::new(root, index as u32))
                })
                .filter(|d| d.pattern().is_none())
                .map(|d| {
                    if let Some(DependencyTarget::External(specifier)) =
                        graph.dependency_target(root, d.id())
                    {
                        return Ok((
                            d,
                            Code::external(
                                specifier.clone(),
                                match d.kind() {
                                    DependencyKind::Import => RequestKind::Import,
                                    DependencyKind::ReExport => RequestKind::ReExport,
                                    DependencyKind::DynamicImport => RequestKind::DynamicImport,
                                    DependencyKind::Require => RequestKind::Require,
                                    DependencyKind::ImportType => {
                                        eyre::bail!("type dependency in executable JavaScript")
                                    }
                                },
                            ),
                            false,
                        ));
                    }
                    let target = target_file(graph, root, d)?;
                    if cjs.contains(target) {
                        Ok((
                            d,
                            if d.kind() == DependencyKind::DynamicImport {
                                cjs.dynamic(graph, target)?
                            } else {
                                cjs.static_dependency(graph, root, target, d, *facts, symbols)?
                            },
                            true,
                        ))
                    } else {
                        Ok((
                            d,
                            Code::request(
                                path(graph, owners[&target]),
                                match d.kind() {
                                    DependencyKind::DynamicImport => RequestKind::DynamicImport,
                                    DependencyKind::ReExport => RequestKind::ReExport,
                                    DependencyKind::Require => RequestKind::Require,
                                    DependencyKind::Import => RequestKind::Import,
                                    DependencyKind::ImportType => {
                                        eyre::bail!("type import in executable JavaScript")
                                    }
                                },
                            ),
                            false,
                        ))
                    }
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            let mut imports = Code::default();
            // Evaluation-only connectors need not own a retained statement.
            for dependency in facts.dependencies().iter().filter(|d| {
                !d.type_only()
                    && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
            }) {
                if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(root, dependency.id())
                {
                    imports.push(
                        Code::external(specifier.clone(), RequestKind::Import)
                            .surrounded("import ", ";\n"),
                    );
                    continue;
                }
                let target = target_file(graph, root, dependency)?;
                if cjs.contains(target) {
                    imports.push(cjs.evaluate(graph, root, target)?);
                } else {
                    imports.push(Code::evaluation(path(graph, owners[&target])));
                }
            }
            for &key in placement.schedule(segments::Owner::Evaluation(root)) {
                eyre::ensure!(
                    key.file_id == root,
                    "relocated segment in unchanged native job"
                );
                let index = key.segment_idx as usize;
                let recipe = facts.emission().segments().get(index).unwrap();
                let mut renderer = crate::emit::CachedRenderer::new(
                    module.source,
                    *facts,
                    recipe,
                    recipes.get(&key).copied().unwrap_or_default(),
                );
                renderer.relocate_urls(module.relocations)?;
                for (dependency, replacement, whole) in &edits {
                    renderer.dependency(*dependency, replacement.clone(), *whole);
                }
                for dependency in facts.dependencies().iter().filter(|d| {
                    d.pattern().is_some()
                        && contains(module.parsed.segments().get(index).unwrap().span, d.span())
                }) {
                    if let Some(dispatch) = cjs.context_dynamic(graph, root, dependency, &owners)? {
                        renderer.context(dependency, dispatch);
                    }
                }
                imports.push(renderer.materialize()?);
                imports.push_str("\n;\n");
            }
            imports.default_export |= facts.exports().iter().any(|export| {
                !export.type_only()
                    && export.name().as_deref() == Some("default")
                    && module
                        .parsed
                        .segments()
                        .iter()
                        .enumerate()
                        .any(|(index, segment)| {
                            live.contains(&SegmentKey::new(root, index as u32))
                                && contains(segment.span, export.span())
                        })
            });
            imports
        };
        let namespace = surfaces
            .contains(&root)
            .then(|| graph.file_path(root).unwrap().to_owned());
        Ok((artifact_path, output, namespace))
    })?;
    let _finalize = tracing::info_span!("emit.finalize").entered();
    for output in outputs {
        let (artifact_path, output, namespace) = output?;
        if let Some(namespace) = namespace {
            namespaces.insert(namespace, artifact_path.clone());
        }
        files.insert(artifact_path, output);
    }
    for (&file, path) in &cjs.entries {
        namespaces.insert(graph.file_path(file).unwrap().to_owned(), path.clone());
    }
    for (path, output) in &cjs.files {
        files.insert(path.clone(), output.clone());
    }
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
                .unwrap_or_else(|| path(graph, owners[&file]))
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
    let removable: BTreeSet<_> = jobs
        .iter()
        .filter(|file| {
            !boundaries.contains(file)
                && !cycles.contains(file)
                && !cjs.contains(**file)
                && placement
                    .owners
                    .iter()
                    .filter(|(_, owner)| **owner == segments::Owner::Evaluation(**file))
                    .all(|(key, _)| {
                        modules[&key.file_id]
                            .parsed
                            .facts()
                            .emission()
                            .segments()
                            .get(key.segment_idx as usize)
                            .unwrap()
                            .connector_only()
                    })
        })
        .map(|&file| path(graph, file))
        .collect();
    Code::prune(&mut files, &removable, &mut members)?;
    namespaces.retain(|_, path| files.contains_key(path));
    for (chunk, nodes) in &plan.chunks {
        chunk_artifacts.insert(
            chunk.clone(),
            nodes
                .iter()
                .filter_map(|node| members.get(node).cloned())
                .collect(),
        );
    }
    cjs.add_runtime(&mut files, &mut chunk_artifacts)?;
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
                        .unwrap_or_else(|| path(graph, owners[&file])),
                )
            })
            .collect(),
        async_modules,
    })
}

fn path(graph: &SourceGraph, file: u32) -> PathBuf {
    PathBuf::from(format!(
        "native/{}.mjs",
        digest(graph.file_path(file).unwrap().to_str().unwrap())
    ))
}

fn request(graph: &SourceGraph, file: u32) -> Code {
    Code::request(path(graph, file), RequestKind::DynamicImport)
}
