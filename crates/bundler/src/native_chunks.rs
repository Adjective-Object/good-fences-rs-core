//! Conservative grouping; native module jobs own every remaining semantic seam.
use super::*;
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
    symbols: &Symbols,
) -> eyre::Result<Artifacts> {
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
                let NodeId::Module(target) = &manifest.async_facades[site].target else {
                    eyre::bail!("nonmodule async target")
                };
                async_targets.insert(site.clone(), graph.file_id(target).unwrap());
            }
        }
    }
    let mut cjs = commonjs::CommonJs::new(graph, plan, modules)?;
    let async_modules = super::super::async_modules(graph, modules)?;
    let asynchronous: BTreeSet<_> = async_modules
        .iter()
        .map(|p| graph.file_id(p).unwrap())
        .collect();
    let mut edges = BTreeMap::new();
    let mut connectors = BTreeSet::new();
    for (&file, module) in modules {
        if module.parsed.facts.commonjs {
            edges.insert(file, Vec::new());
            continue;
        }
        let allocator = Allocator::default();
        rewrite::count_output_parse();
        let parsed = Parser::new(&allocator, module.source, SourceType::mjs()).parse();
        if parsed.program.body.iter().all(|s| match s {
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::EmptyStatement(_) => true,
            Statement::ExportNamedDeclaration(e) => e.declaration.is_none(),
            _ => false,
        }) {
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
                .map(|d| target_file(graph, file, d))
                .collect::<eyre::Result<Vec<_>>>()?,
        );
    }
    let mut boundaries: BTreeSet<_> = entry_owners.values().flatten().copied().collect();
    boundaries.extend(async_targets.values().copied());
    // Keep source wait bounds and sibling interleaving in native module jobs.
    boundaries.extend(&asynchronous);
    boundaries.extend(modules.keys().filter(|&&f| cjs.contains(f)).copied());
    for (&file, module) in modules {
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .filter(|d| !d.type_only && d.kind == DependencyKind::Require)
        {
            boundaries.insert(target_file(graph, file, dependency)?);
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
    // Concatenating even one edge of an SCC can change instantiation/TDZ.
    let files: Vec<_> = modules.keys().copied().collect();
    let indices: BTreeMap<_, _> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let dependencies: Vec<_> = files
        .iter()
        .map(|f| edges[f].iter().map(|t| indices[t]).collect())
        .collect();
    let cycles: BTreeSet<_> = crate::effect_dependencies::components(&dependencies)
        .into_iter()
        .filter(|c| c.len() > 1 || dependencies[c[0]].contains(&c[0]))
        .flatten()
        .map(|i| files[i])
        .collect();
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
    let live = plan.retention.segments.iter().map(|s| s.key).collect();
    let recipes = plan
        .retention
        .effects
        .iter()
        .filter(|e| !e.summary.rewrites.is_empty())
        .map(|e| (e.key, e.summary.rewrites.as_slice()))
        .collect();
    let mut owners = redirects.clone();
    let mut groups: BTreeMap<_, _> = modules
        .keys()
        .filter(|&&f| redirects[&f] == f)
        .map(|&f| (f, BTreeSet::from([f])))
        .collect();
    let mut roots: Vec<_> = modules.keys().copied().collect();
    roots.sort_by_key(|&f| graph.file_path(f));
    loop {
        let mut changed = false;
        for &root in &roots {
            let Some(group) = groups.get(&root) else {
                continue;
            };
            if cycles.contains(&root) || asynchronous.contains(&root) || cjs.contains(root) {
                continue;
            }
            let dependencies: BTreeSet<_> = group
                .iter()
                .flat_map(|f| &edges[f])
                .map(|f| owners[f])
                .filter(|&f| f != root)
                .collect();
            let mut dependencies: Vec<_> = dependencies.into_iter().collect();
            dependencies.sort_by_key(|&f| graph.file_path(f));
            // Exclusive chains preserve completion order. Body-free connectors
            // also disappear among siblings without moving an initializer.
            let Some(&dependency) = dependencies.iter().find(|&&dependency| {
                !boundaries.contains(&dependency)
                    && !cycles.contains(&dependency)
                    && (dependencies.len() == 1
                        || groups[&dependency].iter().all(|f| connectors.contains(f)))
                    && !edges.iter().any(|(file, targets)| {
                        owners[file] != root
                            && owners[file] != dependency
                            && targets.iter().any(|t| owners[t] == dependency)
                    })
            }) else {
                continue;
            };
            let joined = group.union(&groups[&dependency]).copied().collect();
            if concatenate(
                graph, root, &joined, &owners, &redirects, modules, symbols, &live, &recipes, &cjs,
            )?
            .is_none()
            {
                continue;
            }
            for owner in owners.values_mut() {
                if *owner == dependency {
                    *owner = root;
                }
            }
            groups.remove(&dependency);
            groups.insert(root, joined);
            changed = true;
        }
        if !changed {
            break;
        }
    }
    let mut protected = cycles.clone();
    protected.extend(&asynchronous);
    for (&file, module) in modules {
        let mut cjs_dependency = false;
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .filter(|d| !d.type_only)
        {
            if dependency.kind == DependencyKind::Require
                || cjs.contains(target_file(graph, file, dependency)?)
            {
                cjs_dependency = true;
                break;
            }
        }
        if cjs.contains(file)
            || cjs_dependency
            || symbols.imports.iter().any(|(&(source, _), linked)| {
                source == file && matches!(linked.identity, ExportIdentity::Namespace(_))
            })
            || symbols.exports[&file].values().any(|linked| {
                matches!(
                    linked.identity,
                    ExportIdentity::Namespace(_) | ExportIdentity::CommonJs(..)
                )
            })
        {
            protected.insert(file);
        }
    }
    for linked in symbols
        .imports
        .values()
        .chain(symbols.exports.values().flat_map(|e| e.values()))
    {
        if let ExportIdentity::Namespace(file) = linked.identity {
            protected.insert(file);
        }
    }
    protected = protected.into_iter().map(|f| owners[&f]).collect();
    let placement = segments::Placement::new(
        graph,
        plan,
        modules,
        symbols,
        &owners,
        &protected,
        &boundaries,
        &cjs,
    )?;
    let mut surfaces = boundaries.clone();
    surfaces.extend(
        groups
            .keys()
            .filter(|f| !placement.changed.contains(f))
            .copied(),
    );
    let mut files = BTreeMap::new();
    let mut namespaces = BTreeMap::new();
    let entry_files = entry_owners.values().flatten().copied().collect();
    cjs.emit(graph, modules, &owners, &entry_files)?;
    for (&root, group) in &groups {
        if cjs.contains(root) {
            continue;
        }
        let path = path(graph, root);
        let output = if placement.changed.contains(&root) {
            segments::emit(
                graph, root, group, &owners, &redirects, modules, symbols, &placement, &surfaces,
                &recipes,
            )?
        } else if group.len() > 1 {
            concatenate(
                graph, root, group, &owners, &redirects, modules, symbols, &live, &recipes, &cjs,
            )?
            .ok_or_eyre("native group became incompatible")?
        } else {
            let source = retained_source(root, &modules[&root], &live, &recipes)?;
            rewrite::count_output_parse();
            let parsed = ast_segmenter::parse_module(
                &logger::StdioLogger::new(),
                &source,
                graph.file_path(root).unwrap(),
                ast_segmenter::ModuleFormat::Esm,
            )?;
            let edits = parsed
                .facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only)
                .map(|d| {
                    // Retention changes occurrence offsets; resolve by original request.
                    let original = modules[&root]
                        .parsed
                        .facts
                        .dependencies
                        .iter()
                        .find(|o| o.specifier == d.specifier && o.kind == d.kind)
                        .ok_or_eyre("missing retained native dependency")?;
                    let target = target_file(graph, root, original)?;
                    if cjs.contains(target) {
                        Ok(Edit {
                            span: d.span,
                            replacement: if d.kind == DependencyKind::DynamicImport {
                                cjs.dynamic(graph, target)?
                            } else {
                                cjs.static_dependency(
                                    graph,
                                    root,
                                    target,
                                    d,
                                    &parsed.facts,
                                    symbols,
                                )?
                            },
                        })
                    } else {
                        Ok(Edit {
                            span: d.specifier_span,
                            replacement: request(graph, owners[&target]),
                        })
                    }
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            let mut imports = String::new();
            // Evaluation-only connectors need not own a retained statement.
            for &target in &edges[&root] {
                if cjs.contains(target) {
                    imports.push_str(&cjs.evaluate(graph, root, target)?);
                } else {
                    imports.push_str(&format!("import {};\n", request(graph, owners[&target])));
                }
            }
            imports.push_str(&rewrite::apply(&source, &edits)?);
            imports
        };
        let output = rewrite::apply(&output, &rewrite::source_hint_edits(&output))?;
        rewrite::validate(&path, &output)?;
        if surfaces.contains(&root) {
            namespaces.insert(graph.file_path(root).unwrap().to_owned(), path.clone());
        }
        files.insert(path, output);
    }
    for (&file, path) in &cjs.entries {
        namespaces.insert(graph.file_path(file).unwrap().to_owned(), path.clone());
    }
    for (path, output) in &cjs.files {
        let output = rewrite::apply(output, &rewrite::source_hint_edits(output))?;
        rewrite::validate(path, &output)?;
        files.insert(path.clone(), output);
    }
    let mut members = BTreeMap::new();
    let mut chunk_artifacts = BTreeMap::new();
    for (chunk, nodes) in &plan.chunks {
        let mut paths = BTreeSet::new();
        for node in nodes {
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
                NodeId::Segment { index, .. } => placement.owners[&SegmentKey::new(file, *index)],
                _ => owners[&file],
            };
            let owner = path(graph, owner);
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
            if cjs.contains(*file) || !root.binding_roots.is_empty() {
                entry_paths.insert(entry, module_path(*file));
                continue;
            }
        }
        let entry_path = PathBuf::from(format!("native/entry-{}.mjs", digest(&entry)));
        let mut source = String::new();
        for file in roots {
            source.push_str(&format!("import {};\n", specifier(&module_path(file))));
        }
        rewrite::validate(&entry_path, &source)?;
        files.insert(entry_path.clone(), source);
        entry_paths.insert(entry, entry_path);
    }
    Ok(Artifacts {
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

fn request(graph: &SourceGraph, file: u32) -> String {
    quote(&format!(
        "./{}",
        path(graph, file).file_name().unwrap().to_str().unwrap()
    ))
}

#[expect(clippy::too_many_arguments)]
fn concatenate(
    graph: &SourceGraph,
    root: u32,
    group: &BTreeSet<u32>,
    owners: &BTreeMap<u32, u32>,
    redirects: &BTreeMap<u32, u32>,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    live: &BTreeSet<SegmentKey>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
    cjs: &commonjs::CommonJs,
) -> eyre::Result<Option<String>> {
    if group.iter().any(|f| {
        cjs.contains(*f)
            || modules[f]
                .parsed
                .facts
                .dependencies
                .iter()
                .any(|d| !d.type_only && target_file(graph, *f, d).is_ok_and(|t| cjs.contains(t)))
    }) {
        return Ok(None);
    }
    let mut values = Values::new(graph, modules, symbols);
    values.requests = owners
        .iter()
        .map(|(&file, &owner)| (file, request(graph, owner)))
        .collect();
    let mut output = String::new();
    let mut order = Vec::new();
    let mut pending = vec![(root, false)];
    let mut seen = BTreeSet::new();
    while let Some((file, finish)) = pending.pop() {
        if finish {
            order.push(file);
            continue;
        }
        if !seen.insert(file) {
            continue;
        }
        if !group.contains(&file) {
            output.push_str(&format!("import {};\n", request(graph, owners[&file])));
            continue;
        }
        pending.push((file, true));
        for d in modules[&file]
            .parsed
            .facts
            .dependencies
            .iter()
            .rev()
            .filter(|d| {
                !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            })
        {
            pending.push((redirects[&target_file(graph, file, d)?], false));
        }
    }
    let mut occupied: BTreeSet<_> = modules
        .values()
        .flat_map(|m| {
            m.parsed
                .facts
                .bindings
                .iter()
                .map(|b| b.name.clone())
                .chain(m.parsed.facts.references.iter().map(|r| r.name.clone()))
        })
        .chain(symbols.definitions.values().map(|d| d.name.clone()))
        .chain(values.defaults.values().cloned())
        .collect();
    let mut fresh = 0;
    for &file in &order {
        let facts = &modules[&file].parsed.facts;
        for import in &facts.imports {
            if !symbols.imports.contains_key(&(file, import.binding)) {
                continue;
            }
            let dependency = facts
                .dependencies
                .iter()
                .find(|d| d.id == import.dependency)
                .unwrap();
            let target = redirects[&target_file(graph, file, dependency)?];
            if group.contains(&target) {
                if facts
                    .references
                    .iter()
                    .any(|r| r.write && r.binding == Some(import.binding))
                {
                    return Ok(None);
                }
                continue;
            }
            let alias = loop {
                let name = format!("__gf_import_{fresh}");
                fresh += 1;
                if occupied.insert(name.clone()) {
                    break name;
                }
            };
            let specifier = match &import.imported {
                ImportedName::Name(name) => format!("{{{} as {alias}}}", quote(name)),
                ImportedName::Namespace => format!("* as {alias}"),
            };
            output.push_str(&format!(
                "import {specifier} from {};\n",
                request(graph, owners[&target])
            ));
            values.imports.insert((file, import.binding), alias);
        }
    }
    let mut bodies = String::new();
    for &file in &order {
        let Some((hoisted, body)) = body(graph, file, &modules[&file], &values, live, recipes)?
        else {
            return Ok(None);
        };
        output.push_str(&hoisted);
        bodies.push_str(&body);
        bodies.push_str("\n;\n");
    }
    output.push_str(&bodies);
    for (public, linked) in &symbols.exports[&root] {
        let file = match linked.identity {
            ExportIdentity::Binding(file, _) | ExportIdentity::Namespace(file) => file,
            ExportIdentity::Expression(key) => key.file_id,
            _ => return Ok(None),
        };
        if group.contains(&file) {
            let Some(name) = values.name(&linked.identity) else {
                return Ok(None);
            };
            output.push_str(&format!("export {{{name} as {}}};\n", quote(public)));
        } else {
            let owner = owners[&file];
            let request = request(graph, owner);
            if matches!(linked.identity, ExportIdentity::Namespace(_)) {
                output.push_str(&format!("export * as {} from {request};\n", quote(public)));
            } else {
                let name = symbols.exports[&owner]
                    .iter()
                    .find(|(_, e)| e.identity == linked.identity)
                    .map(|(n, _)| n)
                    .ok_or_eyre("native cross-chunk export is absent")?;
                output.push_str(&format!(
                    "export {{{} as {}}} from {request};\n",
                    quote(name),
                    quote(public)
                ));
            }
        }
    }
    Ok(Some(output))
}
