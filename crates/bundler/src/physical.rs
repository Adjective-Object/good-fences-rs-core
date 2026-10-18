//! Physical ownership and native evaluation jobs are deliberately separate.
use super::*;
use ast_segmenter::owned::ImportedName;
use source_graph::ExportIdentity;

#[path = "physical_lower.rs"]
mod lower;

const RUNTIME: &str = "__gf_runtime";

fn id(graph: &SourceGraph, file: u32) -> String {
    digest(graph.file_path(file).unwrap().to_str().unwrap())
}

fn key(graph: &SourceGraph, file: u32, name: &str) -> String {
    quote(&format!("{}:{name}", id(graph, file)))
}

fn binding(graph: &SourceGraph, file: u32, name: &str) -> String {
    format!("{RUNTIME}.cell({}).v", key(graph, file, name))
}

fn export_value(graph: &SourceGraph, file: u32, name: &str) -> eyre::Result<String> {
    let identity = graph.resolve_export(file, name, Namespace::Value)?.identity;
    Ok(match identity {
        ExportIdentity::Binding(file, binding_id) => {
            let definition = graph
                .file_facts(file)
                .unwrap()
                .bindings
                .iter()
                .find(|b| b.id == binding_id)
                .ok_or_eyre("missing export binding")?;
            binding(graph, file, &definition.name)
        }
        ExportIdentity::Expression(segment) => binding(graph, segment.file_id, "*default*"),
        ExportIdentity::Namespace(file) => {
            format!("{RUNTIME}.namespace({})", quote(&id(graph, file)))
        }
        ExportIdentity::CommonJs(file, name) => format!(
            "{RUNTIME}.proxy({}, {})",
            quote(&id(graph, file)),
            quote(if name == "default" { "" } else { &name })
        ),
        ExportIdentity::External(..) => eyre::bail!("external export emission has no policy"),
    })
}

fn imported_value(
    graph: &SourceGraph,
    file: u32,
    import: &ast_segmenter::owned::ImportBinding,
) -> eyre::Result<String> {
    let dependency = graph
        .file_facts(file)
        .unwrap()
        .dependencies
        .iter()
        .find(|d| d.id == import.dependency)
        .unwrap();
    let target = target_file(graph, file, dependency)?;
    match &import.imported {
        ImportedName::Name(name) => export_value(graph, target, name),
        ImportedName::Namespace => Ok(format!(
            "{RUNTIME}.namespace({})",
            quote(&id(graph, target))
        )),
    }
}

fn proxy_facade(file: u32, name: &str, graph: &SourceGraph) -> PathBuf {
    PathBuf::from(format!("facades/{}-{}.mjs", id(graph, file), digest(name)))
}

fn namespace_facade(graph: &SourceGraph, file: u32) -> PathBuf {
    PathBuf::from(format!("facades/{}.namespace.mjs", id(graph, file)))
}

fn relative(path: &std::path::Path) -> String {
    quote(&format!("../{}", path.display()))
}

struct Captures<'a> {
    graph: &'a SourceGraph,
    plan: &'a ChunkPlan,
    chunks: &'a BTreeMap<ChunkId, PathBuf>,
    modules: &'a BTreeMap<u32, Module<'a>>,
}

impl Captures<'_> {
    fn emit(
        &self,
        load: &BTreeSet<ChunkId>,
        evaluation: &[u32],
        namespace: Option<u32>,
        path: PathBuf,
        files: &mut BTreeMap<PathBuf, String>,
    ) -> eyre::Result<()> {
        let Self {
            graph,
            plan,
            chunks,
            modules,
        } = *self;
        let mut active = BTreeSet::new();
        let mut pending = evaluation.to_vec();
        while let Some(file) = pending.pop() {
            if !active.insert(file) || modules[&file].parsed.facts.commonjs {
                continue;
            }
            for dependency in &modules[&file].parsed.facts.dependencies {
                if !dependency.type_only
                    && matches!(
                        dependency.kind,
                        DependencyKind::Import | DependencyKind::ReExport
                    )
                {
                    pending.push(target_file(graph, file, dependency)?);
                }
            }
        }
        let mut namespaces = BTreeSet::new();
        if let Some(file) = namespace {
            if !modules[&file].parsed.facts.commonjs {
                namespaces.insert(file);
            }
        }
        for edge in &plan.retention.edges {
            if edge.kind != crate::RetentionEdgeKind::Binding {
                continue;
            }
            let consumer = match edge.from {
                GraphNode::Module(file) => file,
                GraphNode::Segment(segment) => segment.file_id,
                _ => continue,
            };
            if !active.contains(&consumer) {
                continue;
            }
            let GraphNode::Module(target) = edge.to else {
                continue;
            };
            if modules[&target].parsed.facts.commonjs {
                continue;
            }
            if let Some(index) = edge.source_edge {
                let source = &graph.edges()[index];
                if source.activation == source_graph::Activation::Asynchronous || source.type_only {
                    continue;
                }
            }
            if plan
                .chunk_for(&crate::chunks::node_id(graph, edge.from)?)
                .is_some_and(|chunk| load.contains(&chunk))
            {
                namespaces.insert(target);
            }
        }
        let mut namespaces: Vec<_> = namespaces.into_iter().collect();
        namespaces.sort_by_key(|&file| graph.file_path(file));
        let register = path.with_extension("capture-register.mjs");
        let mut source: String = load
            .iter()
            .map(|chunk| format!("import {};\n", relative(&chunks[chunk])))
            .collect();
        source.push_str(&format!(
            "import {};\nimport {RUNTIME} from '../runtime.cjs';\n",
            specifier(&register)
        ));
        // GetNamespace is hoisted: the registration cycle can expose native
        // namespace objects without evaluating their modules ahead of source order.
        for &file in evaluation {
            if !modules[&file].parsed.facts.commonjs {
                source.push_str(&format!("import {};\n", specifier(&modules[&file].path)));
            }
        }
        for (index, &file) in namespaces.iter().enumerate() {
            source.push_str(&format!(
                "import * as n{index} from {};\n",
                specifier(&namespace_facade(graph, file))
            ));
        }
        let values: Vec<_> = namespaces
            .iter()
            .enumerate()
            .map(|(index, &file)| format!("[{},n{index}]", quote(&id(graph, file))))
            .collect();
        source.push_str(&format!(
            "export function getNamespaces() {{return [{}];}}\n",
            values.join(",")
        ));
        source.push_str(&format!("{RUNTIME}.completeProxies();\n"));
        for &file in evaluation {
            if !modules[&file].parsed.facts.commonjs {
                source.push_str(&format!("{RUNTIME}.finish({});\n", quote(&id(graph, file))));
            }
        }
        files.insert(register,format!(
        "import {{getNamespaces}} from {};\nimport {RUNTIME} from '../runtime.cjs';\nfor (const [id,namespace] of getNamespaces()) {RUNTIME}.expose(id,namespace);\n",
        specifier(&path)
    ));
        files.insert(path, source);
        Ok(())
    }
}

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: BTreeMap<u32, Module<'_>>,
    manifest: DiagnosticManifest,
) -> eyre::Result<Artifacts> {
    let async_modules = super::async_modules(graph, &modules)?;
    let mut chunk_runtime = RUNTIME.to_owned();
    while modules.values().any(|module| {
        module
            .parsed
            .facts
            .bindings
            .iter()
            .map(|binding| &binding.name)
            .chain(
                module
                    .parsed
                    .facts
                    .references
                    .iter()
                    .map(|reference| &reference.name),
            )
            .any(|name| name == &chunk_runtime)
    }) {
        chunk_runtime.push('$');
    }
    let chunks: BTreeMap<_, _> = plan
        .chunks
        .keys()
        .map(|chunk| {
            (
                chunk.clone(),
                PathBuf::from(format!("chunks/{}.cjs", digest(&format!("{:?}", chunk.0)))),
            )
        })
        .collect();
    let mut files = BTreeMap::new();
    let mut members = BTreeMap::new();
    let mut chunk_artifacts = BTreeMap::new();
    for (chunk, nodes) in &plan.chunks {
        let path = &chunks[chunk];
        chunk_artifacts.insert(chunk.clone(), BTreeSet::from([path.clone()]));
        files.insert(
            path.clone(),
            format!("const {chunk_runtime} = require('../runtime.cjs');\n"),
        );
        for node in nodes {
            ensure!(
                !matches!(node, NodeId::External { .. }),
                "external chunk emission needs a declared policy: {node:?}"
            );
            members.insert(node.clone(), path.clone());
        }
    }
    let append = |files: &mut BTreeMap<PathBuf, String>, node: NodeId, source: String| {
        files.get_mut(&members[&node]).unwrap().push_str(&source);
    };
    let mut cjs_proxies = BTreeMap::new();
    let mut cjs_namespaces = BTreeMap::new();
    let mut hoisted_defaults = BTreeSet::new();
    let mut ordered: Vec<_> = modules.iter().collect();
    ordered.sort_by_key(|(&file, _)| graph.file_path(file));
    for &(&file, module) in &ordered {
        let original_path = graph.file_path(file).unwrap().to_owned();
        let module_id = quote(&id(graph, file));
        append(
            &mut files,
            NodeId::Module(original_path.clone()),
            format!(
                "{chunk_runtime}.moduleRecord({module_id}, {});\n",
                module.parsed.facts.commonjs
            ),
        );
        if module.parsed.facts.commonjs {
            let mut edits = Vec::new();
            if let Some(hashbang) = &module.parsed.facts.hashbang {
                edits.push(crate::rewrite::Edit {
                    span: hashbang.span,
                    replacement: String::new(),
                });
            }
            for dependency in module
                .parsed
                .facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only)
            {
                let replacement = if dependency.kind == DependencyKind::DynamicImport {
                    dynamic_import(graph, file, dependency)
                } else if dependency.kind == DependencyKind::Require {
                    let target = target_file(graph, file, dependency)?;
                    format!("require({})", quote(&id(graph, target)))
                } else {
                    eyre::bail!("static ESM dependency in CommonJS factory");
                };
                edits.push(crate::rewrite::Edit {
                    span: dependency.span,
                    replacement,
                });
            }
            let source = crate::rewrite::apply(module.source, &edits)?;
            let source =
                crate::rewrite::apply(&source, &crate::rewrite::source_hint_edits(&source))?;
            append(
                &mut files,
                NodeId::Segment { module: original_path.clone(), index: 0 },
                format!("{chunk_runtime}.factory({module_id}, function(exports, require, module) {{\n{source}\n}}, false);\n"),
            );
            for (node, path) in &members {
                let (name, code) = match node {
                    NodeId::CommonJsProxy {
                        module: target,
                        name,
                    } if target == &original_path => {
                        let proxy_name = name.as_deref().unwrap_or("");
                        let code = if let Some(name) = name {
                            format!("const value = {chunk_runtime}.proxy({module_id}, ''); return value == null ? undefined : value[{}];", quote(name))
                        } else {
                            format!("return {chunk_runtime}.requireModule({module_id});")
                        };
                        (proxy_name, code)
                    }
                    NodeId::CommonJsNamespace(target) if target == &original_path => (
                        "*",
                        format!(
                            "{}\nreturn commonjsNamespace({chunk_runtime}.proxy({module_id}, ''));",
                            include_str!("commonjs_namespace.mjs")
                        ),
                    ),
                    _ => continue,
                };
                files.get_mut(path).unwrap().push_str(&format!(
                    "{chunk_runtime}.proxyFactory({module_id}, {}, function() {{{code}}});\n",
                    quote(name)
                ));
                let facade = proxy_facade(file, name, graph);
                files.insert(facade.clone(), format!(
                    "import {RUNTIME} from '../runtime.cjs';\nlet value;\nexport {{value as default}};\n{RUNTIME}.watch(()=>{RUNTIME}.proxyValue({module_id}, {}), next => value = next);\n{RUNTIME}.proxyAvailable({module_id}, {});\n", quote(name),quote(name)
                ));
                if name == "*" {
                    cjs_namespaces.insert(original_path.clone(), facade);
                } else if name.is_empty() {
                    cjs_proxies.insert(original_path.clone(), (facade, module.surface.clone()));
                }
            }
        } else {
            let lowered = lower::lower(graph, plan, file, module, &chunk_runtime)?;
            if lowered.hoisted_default {
                hoisted_defaults.insert(file);
            }
            for (index, code) in lowered.segments {
                append(
                    &mut files,
                    NodeId::Segment {
                        module: original_path.clone(),
                        index,
                    },
                    code,
                );
            }
            // Synchronous CJS require enters the same native evaluation job.
            append(
                &mut files,
                NodeId::Module(original_path),
                format!(
                    "{chunk_runtime}.factory({module_id}, () => require({}), true);\n",
                    relative(&namespace_facade(graph, file))
                ),
            );
        }
    }
    for dependency in &manifest.chunk_dependencies {
        let from = &chunks[&dependency.from];
        let to = &chunks[&dependency.to];
        files
            .get_mut(from)
            .unwrap()
            .push_str(&format!("require({});\n", super::specifier(to)));
    }
    for &(&file, module) in ordered
        .iter()
        .filter(|(_, module)| module.parsed.facts.commonjs)
    {
        let original = graph.file_path(file).unwrap();
        let body_chunks: BTreeSet<_> = members
            .iter()
            .filter_map(|(node, path)| match node {
                NodeId::Module(pathname)
                | NodeId::Segment {
                    module: pathname, ..
                } if pathname == original => Some(path),
                _ => None,
            })
            .collect();
        let mut source: String = body_chunks
            .iter()
            .map(|path| format!("require({});\n", relative(path)))
            .collect();
        source.push_str(&format!(
            "module.exports = require('../runtime.cjs').requireModule({});\n",
            quote(&id(graph, file))
        ));
        files.insert(module.path.clone(), source);
        if cjs_proxies.contains_key(original) {
            let path = PathBuf::from(format!("facades/{}.proxy.mjs", id(graph, file)));
            let proxies: BTreeMap<_, _> = members
                .iter()
                .filter_map(|(node, path)| match node {
                    NodeId::CommonJsProxy { module, name } if module == original => {
                        Some((name.clone(), path))
                    }
                    _ => None,
                })
                .collect();
            let load: BTreeSet<_> = proxies.values().collect();
            let mut source: String = load
                .iter()
                .map(|chunk| format!("import {};\n", relative(chunk)))
                .collect();
            for name in proxies.keys() {
                source.push_str(&format!(
                    "export {{default as {}}} from {};\n",
                    quote(name.as_deref().unwrap_or("default")),
                    specifier(&proxy_facade(file, name.as_deref().unwrap_or(""), graph))
                ));
            }
            files.insert(path.clone(), source);
            cjs_proxies.get_mut(original).unwrap().0 = path;
        }
    }
    for &(&file, module) in &ordered {
        if module.parsed.facts.commonjs {
            continue;
        }
        let register = PathBuf::from(format!("facades/{}.bindings.mjs", id(graph, file)));
        let mut source = format!(
            "import {RUNTIME} from '../runtime.cjs';\nimport {};\n",
            specifier(&register)
        );
        for dependency in module.parsed.facts.dependencies.iter().filter(|d| {
            !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
        }) {
            let target = target_file(graph, file, dependency)?;
            let target_module = &modules[&target];
            if target_module.parsed.facts.commonjs {
                source.push_str(&format!(
                    "import {};\n",
                    specifier(&proxy_facade(target, "", graph))
                ));
                // Snapshots belong to their own exact-set nodes, not the opaque body.
                let mut requests = BTreeSet::new();
                for (from, request) in crate::cjs::requests(graph, &plan.retention, target) {
                    let consumer = match from {
                        GraphNode::Module(f) => f,
                        GraphNode::Segment(s) => s.file_id,
                        GraphNode::Unlinked { file_id, .. } => file_id,
                    };
                    if consumer == file {
                        requests.insert(match request {
                            ImportedName::Name(name) if name == "default" => String::new(),
                            ImportedName::Name(name) => name,
                            ImportedName::Namespace => "*".into(),
                        });
                    }
                }
                for name in requests.into_iter().filter(|n| !n.is_empty()) {
                    source.push_str(&format!(
                        "import {};\n",
                        specifier(&proxy_facade(target, &name, graph))
                    ));
                }
            } else {
                source.push_str(&format!("import {};\n", specifier(&target_module.path)));
            }
        }
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for name in &module.surface {
            groups
                .entry(export_value(graph, file, name)?)
                .or_default()
                .push(name.clone());
        }
        let mut lexical = Vec::new();
        let mut subscriptions = format!("import {RUNTIME} from '../runtime.cjs';\n");
        for (index, (expression, names)) in groups.into_iter().enumerate() {
            let identity = graph
                .resolve_export(file, &names[0], Namespace::Value)?
                .identity;
            let (owner, own) = match &identity {
                ExportIdentity::Binding(owner, binding_id) => {
                    let definition = graph
                        .file_facts(*owner)
                        .unwrap()
                        .bindings
                        .iter()
                        .find(|b| b.id == *binding_id)
                        .unwrap();
                    (
                        *owner,
                        Some((
                            definition.name.clone(),
                            binding_id.0,
                            definition.function || definition.function_scoped,
                        )),
                    )
                }
                ExportIdentity::Expression(segment) => (
                    segment.file_id,
                    Some((
                        "*default*".into(),
                        graph.file_segments(segment.file_id).unwrap()[segment.segment_idx as usize]
                            .span
                            .start,
                        hoisted_defaults.contains(&segment.file_id),
                    )),
                ),
                ExportIdentity::Namespace(_) => (file, Some((String::new(), 0, true))),
                ExportIdentity::CommonJs(target, name) => {
                    for public in names {
                        source.push_str(&format!(
                            "export {{default as {}}} from {};\n",
                            quote(&public),
                            specifier(&proxy_facade(
                                *target,
                                if name == "default" { "" } else { name },
                                graph
                            ))
                        ));
                    }
                    continue;
                }
                ExportIdentity::External(..) => eyre::bail!("external export has no output policy"),
            };
            if owner != file {
                let original = modules[&owner]
                    .surface
                    .iter()
                    .find(|name| {
                        graph
                            .resolve_export(owner, name, Namespace::Value)
                            .is_ok_and(|e| e.identity == identity)
                    })
                    .ok_or_eyre("export identity has no original public name")?;
                for name in names {
                    source.push_str(&format!(
                        "export {{{} as {}}} from {};\n",
                        quote(original),
                        quote(&name),
                        specifier(&modules[&owner].path)
                    ));
                }
                continue;
            }
            let (binding_name, position, var) = own.unwrap();
            for name in names {
                source.push_str(&format!("export {{e{index} as {}}};\n", quote(&name)));
            }
            let mut setter = format!("__gf_setter_{index}");
            while module.surface.contains(&setter) {
                setter.push('$');
            }
            source.push_str(&format!(
                "export function {setter}(value) {{e{index} = value;}}\n"
            ));
            subscriptions.push_str(&format!(
                "import {{{setter}}} from {};\n{RUNTIME}.watch(()=>({expression}),{setter});\n",
                specifier(&module.path)
            ));
            if var {
                source.push_str(&format!("var e{index};\n"));
            } else {
                lexical.push((position, index, key(graph, file, &binding_name)));
            }
        }
        files.insert(register, subscriptions);
        let module_id = quote(&id(graph, file));
        source.push_str(&format!(
            "const execution = {RUNTIME}.execute({module_id});\nlet step = execution.next();\n"
        ));
        let advance = if module.parsed.facts.top_level_await.is_empty() {
            "if ('await' in step.value) throw Error('unexpected async segment');\nstep = execution.next();\n".to_owned()
        } else {
            "if ('await' in step.value) {\nlet value;\ntry {value = await step.value.await;} catch(error) {step = execution.throw(error); continue;}\nstep = execution.next(value);\n} else {step = execution.next();}\n".into()
        };
        lexical.sort_by_key(|&(position, index, _)| (position, index));
        for (_, index, key) in lexical {
            source.push_str(&format!(
                "if ({RUNTIME}.cell({key}).declared) {{while (!step.done && step.value.init !== {key}) {{\n{advance}}}}}\nlet e{index} = {RUNTIME}.initial({key});\nif (!step.done && step.value.init === {key}) step = execution.next();\n"
            ));
        }
        source.push_str(&format!("while (!step.done) {{\n{advance}}}\n"));
        files.insert(module.path.clone(), source);
    }
    // Namespace demand reaches every retained part of its target. Loading this
    // facade is therefore permitted to load their synchronous chunk closure;
    // ordinary named consumers still use the body-free evaluation facade.
    for &(&file, module) in &ordered {
        let original = graph.file_path(file).unwrap().to_owned();
        if module.parsed.facts.commonjs
            && !members.contains_key(&NodeId::CommonJsNamespace(original.clone()))
        {
            continue;
        }
        let mut load: BTreeSet<_> = members
            .keys()
            .filter_map(|node| {
                let owner = match node {
                    NodeId::Module(p)
                    | NodeId::Segment { module: p, .. }
                    | NodeId::CommonJsProxy { module: p, .. }
                    | NodeId::CommonJsNamespace(p) => p,
                    NodeId::External { .. } => return None,
                };
                if module.parsed.facts.commonjs
                    && matches!(node, NodeId::CommonJsProxy { name: Some(_), .. })
                {
                    return None;
                }
                (owner == &original).then(|| plan.chunk_for(node).unwrap())
            })
            .collect();
        loop {
            let old = load.len();
            for edge in &manifest.chunk_dependencies {
                if load.contains(&edge.from) {
                    load.insert(edge.to.clone());
                }
            }
            if load.len() == old {
                break;
            }
        }
        let path = namespace_facade(graph, file);
        let loader = PathBuf::from(format!("facades/{}.capture.mjs", id(graph, file)));
        Captures {
            graph,
            plan,
            chunks: &chunks,
            modules: &modules,
        }
        .emit(&load, &[file], Some(file), loader.clone(), &mut files)?;
        let mut source = format!("import {};\n", specifier(&loader));
        if module.parsed.facts.commonjs {
            source.push_str(&format!(
                "export {{default}} from {};\n",
                specifier(&proxy_facade(file, "*", graph))
            ));
            cjs_namespaces.insert(original, path.clone());
        } else {
            source.push_str(&format!("import {};\n", specifier(&module.path)));
            for name in &module.surface {
                source.push_str(&format!(
                    "export {{{}}} from {};\n",
                    quote(name),
                    specifier(&module.path)
                ));
            }
        }
        files.insert(path, source);
    }
    let mut entries = BTreeMap::new();
    let mut async_facades = BTreeMap::new();
    for (root, seed) in &plan.roots {
        if let RootId::Async(site) = root {
            let NodeId::Module(target) = &manifest.async_facades[site].target else {
                eyre::bail!("nonmodule async target")
            };
            async_facades.insert(
                site.clone(),
                namespace_facade(graph, graph.file_id(target).unwrap()),
            );
            continue;
        }
        let path = match root {
            RootId::Entry(name) => PathBuf::from(format!(
                "facades/entry-{}.{}",
                digest(name),
                if seed.evaluation_roots.len() == 1
                    && modules[&seed.evaluation_roots[0]].parsed.facts.commonjs
                {
                    "cjs"
                } else {
                    "mjs"
                }
            )),
            RootId::Async(_) => unreachable!(),
        };
        let cjs = path.extension().is_some_and(|e| e == "cjs");
        let facade = match root {
            RootId::Entry(name) => &manifest.entries[name],
            RootId::Async(site) => &manifest.async_facades[site].facade,
        };
        let mut source = String::new();
        for chunk in &facade.load_chunks {
            source.push_str(&if cjs {
                format!("require({});\n", relative(&chunks[chunk]))
            } else {
                format!("import {};\n", relative(&chunks[chunk]))
            });
        }
        let evaluation: BTreeSet<_> = seed
            .evaluation_roots
            .iter()
            .copied()
            .chain(seed.binding_roots.iter().map(|n| match n {
                GraphNode::Module(f) => *f,
                GraphNode::Segment(s) => s.file_id,
                GraphNode::Unlinked { file_id, .. } => *file_id,
            }))
            .collect();
        if evaluation.len() == 1 {
            let file = *evaluation.first().unwrap();
            if modules[&file].parsed.facts.commonjs && !cjs {
                let RootId::Entry(name) = root else {
                    unreachable!()
                };
                entries.insert(name.clone(), namespace_facade(graph, file));
                continue;
            }
            if !modules[&file].parsed.facts.commonjs
                && seed.binding_roots.contains(&GraphNode::Module(file))
            {
                let RootId::Entry(name) = root else {
                    unreachable!()
                };
                entries.insert(name.clone(), namespace_facade(graph, file));
                continue;
            }
        }
        if !cjs {
            let mut evaluation: Vec<_> = evaluation.iter().copied().collect();
            evaluation.sort_by_key(|&file| graph.file_path(file));
            let loader = PathBuf::from(format!(
                "facades/entry-{}.capture.mjs",
                digest(match root {
                    RootId::Entry(name) => name,
                    RootId::Async(_) => unreachable!(),
                })
            ));
            Captures {
                graph,
                plan,
                chunks: &chunks,
                modules: &modules,
            }
            .emit(
                &facade.load_chunks,
                &evaluation,
                None,
                loader.clone(),
                &mut files,
            )?;
            source = format!("import {};\n", specifier(&loader));
        }
        for &file in &evaluation {
            let module = &modules[&file];
            let module_id = quote(&id(graph, file));
            if cjs {
                source.push_str(&format!(
                    "module.exports = require('../runtime.cjs').requireModule({module_id});\n"
                ));
            } else if module.parsed.facts.commonjs {
                source.push_str(&format!(
                    "import value from {};\nexport default value;\n",
                    specifier(&proxy_facade(file, "*", graph))
                ));
            } else {
                source.push_str(&format!("import {};\n", specifier(&module.path)));
                if evaluation.len() == 1 {
                    for name in &module.surface {
                        source.push_str(&format!(
                            "export {{{}}} from {};\n",
                            quote(name),
                            specifier(&module.path)
                        ));
                    }
                }
            }
        }
        files.insert(path.clone(), source);
        match root {
            RootId::Entry(name) => {
                entries.insert(name.clone(), path);
            }
            RootId::Async(site) => {
                async_facades.insert(site.clone(), path);
            }
        }
    }
    files.insert(
        "runtime.cjs".into(),
        include_str!("chunk_runtime.cjs").into(),
    );
    for (path, source) in &mut files {
        *source = crate::rewrite::apply(source, &crate::rewrite::source_hint_edits(source))?;
        crate::rewrite::validate(path, source)?;
    }
    Ok(Artifacts {
        files,
        members,
        chunk_artifacts,
        manifest,
        modules: modules
            .iter()
            .map(|(&file, module)| {
                let source = graph.file_path(file).unwrap().to_owned();
                let executable = if module.parsed.facts.commonjs {
                    plan.roots
                        .iter()
                        .find_map(|(root, seed)| {
                            let RootId::Entry(name) = root else {
                                return None;
                            };
                            (seed.evaluation_roots == [file]
                                || seed.binding_roots.iter().any(|n| {
                                    matches!(n,GraphNode::Module(f) if *f == file)
                                        || matches!(n,GraphNode::Segment(s) if s.file_id == file)
                                }))
                            .then(|| entries[name].clone())
                        })
                        .unwrap_or_else(|| module.path.clone())
                } else {
                    namespace_facade(graph, file)
                };
                (source, executable)
            })
            .collect(),
        entries,
        cjs_proxies,
        cjs_namespaces,
        async_facades,
        async_modules,
    })
}

fn dynamic_import(graph: &SourceGraph, file: u32, dependency: &Dependency) -> String {
    let target = target_file(graph, file, dependency).expect("checked dynamic dependency");
    let import = format!("import({})", relative(&namespace_facade(graph, target)));
    if graph.file_facts(target).unwrap().commonjs {
        format!("{import}.then(namespace => namespace.default)")
    } else {
        import
    }
}
