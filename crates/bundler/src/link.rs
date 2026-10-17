//! Exact-chunk source payloads linked as native ESM through Node's loader hooks.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ast_segmenter::owned::{Dependency, DependencyKind, ExportValue, ImportedName};
use ast_segmenter::{ModuleBody, ParsedModule};
use eyre::{ensure, OptionExt, WrapErr};
use oxc_span::Span;
use sha2::{Digest, Sha256};
use source_graph::{DependencyTarget, GraphNode, Namespace, SegmentKey, SourceGraph};

use crate::chunks::{node_id, ChunkId, ChunkPlan, NodeId, RootId};
use crate::manifest::{diagnostic_manifest, DiagnosticManifest};
use crate::policy::EffectPolicy;
use crate::rewrite::{self, Edit};

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

/// Owned staging result; no filesystem writes. Publish the complete relative
/// artifact map together, then use runtime.mjs's loadEntry(name) for the native
/// module namespace, or execute an entry launcher with Node >=22.15.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifacts {
    pub files: BTreeMap<PathBuf, String>,
    pub entries: BTreeMap<String, PathBuf>,
    pub runtime: PathBuf,
    /// Source identities to canonical virtual-module tokens, not physical files.
    pub modules: BTreeMap<PathBuf, String>,
    /// Physical assembly dependencies, distinct from logical root reachability.
    pub module_payloads: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Exactly the P5 nonempty members, each assigned to its exact chunk payload.
    pub members: BTreeMap<NodeId, PathBuf>,
    pub chunk_artifacts: BTreeMap<ChunkId, BTreeSet<PathBuf>>,
    pub manifest: DiagnosticManifest,
    /// CJS identity -> canonical native ESM proxy token and its static surface.
    pub cjs_proxies: BTreeMap<PathBuf, (String, BTreeSet<String>)>,
}

struct Module<'a> {
    source: &'a str,
    parsed: ParsedModule,
    token: String,
    surface: BTreeSet<String>,
}

/// Native ESM owns scope, instantiation-before-evaluation, cycles, lexical TDZ
/// and live exports. Chunks hold rewritten source pieces, never eager factories:
/// the host loader joins one original lexical module before compiling it.
pub fn esm(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Artifacts> {
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    let mut modules = BTreeMap::new();
    for &file in &plan.retention.modules {
        let path = graph.file_path(file).ok_or_eyre("missing linked module")?;
        let source = sources
            .get(path)
            .ok_or_else(|| eyre::eyre!("{}: missing linked source text", path.display()))?;
        let parsed = crate::emit::checked_source(graph, file, source)?;
        let identity = path
            .to_str()
            .ok_or_eyre("emission module paths must be UTF-8")?;
        modules.insert(
            file,
            Module {
                source,
                parsed,
                token: format!("good-fences:{}", digest(identity)),
                surface: BTreeSet::new(),
            },
        );
    }
    let names = export_names(graph, plan);
    for (&file, module) in &mut modules {
        if module.parsed.facts.commonjs {
            module.surface = names[&file].clone();
            continue;
        }
        for name in &names[&file] {
            let linked = match graph.resolve_export(file, name, Namespace::Value) {
                Ok(linked) => linked,
                Err(error)
                    if !module
                        .parsed
                        .facts
                        .exports
                        .iter()
                        .any(|e| e.name.as_ref() == Some(name)) =>
                {
                    // ResolveExport excludes ambiguous/cycle-only star names from
                    // namespaces. Explicit/imported names are checked, never skipped.
                    ensure!(
                        error.to_string().starts_with("ambiguous ")
                            || error.to_string().starts_with("missing "),
                        "{error:#}"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            if linked.route.iter().all(|node| retained(plan, *node)) {
                module.surface.insert(name.clone());
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut members = BTreeMap::new();
    let mut chunk_artifacts = BTreeMap::new();
    let mut payloads: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
    for (chunk, nodes) in &plan.chunks {
        let path = PathBuf::from(format!("chunks/{}.json", digest(&format!("{chunk:?}"))));
        for node in nodes {
            if let NodeId::External { module, offset } = node {
                let span = graph
                    .file_id(module)
                    .and_then(|file| graph.file_facts(file))
                    .and_then(|f| f.dependencies.iter().find(|d| d.id.0 == *offset))
                    .map(|d| d.span)
                    .unwrap_or(Span::new(*offset, *offset));
                eyre::bail!(
                    "{}: external chunk emission needs a declared policy at bytes {}..{}",
                    module.display(),
                    span.start,
                    span.end
                );
            }
            members.insert(node.clone(), path.clone());
        }
        chunk_artifacts.insert(chunk.clone(), BTreeSet::from([path.clone()]));
        payloads.insert(path, BTreeMap::new());
    }
    let mut module_pieces = BTreeMap::new();
    let mut module_payloads = BTreeMap::new();
    let mut cjs_proxies = BTreeMap::new();
    for (&file, module) in &modules {
        let edits = dependency_edits(graph, plan, file, &modules)?;
        let parts = if matches!(module.parsed.body, ModuleBody::CommonJs { .. }) {
            let source = rewrite::apply(module.source, &edits)?;
            vec![crate::emit::SourcePart {
                segment: Some(0),
                source: rewrite::apply(&source, &rewrite::source_hint_edits(&source))?,
            }]
        } else {
            crate::emit::project_parts(
                graph,
                &plan.retention,
                file,
                module.source,
                &module.parsed,
                &edits,
            )
            .wrap_err_with(|| {
                format!(
                    "{}: linked emission",
                    graph.file_path(file).unwrap().display()
                )
            })?
        };
        let mut pieces = Vec::new();
        for (index, part) in parts.into_iter().enumerate() {
            if part.source.is_empty() {
                continue;
            }
            let node = match part.segment {
                Some(index) => GraphNode::Segment(SegmentKey::new(file, index)),
                None => GraphNode::Module(file),
            };
            let path = &members[&node_id(graph, node)?];
            let key = format!("{}:{index}", module.token);
            payloads
                .get_mut(path)
                .unwrap()
                .insert(key.clone(), part.source);
            pieces.push((path.clone(), key));
        }
        module_payloads.insert(
            graph.file_path(file).unwrap().to_owned(),
            pieces.iter().map(|(path, _)| path.clone()).collect(),
        );
        module_pieces.insert(module.token.clone(), pieces);
        if module.parsed.facts.commonjs {
            let proxy = proxy_token(module);
            let mut source = format!(
                "import {{requireModule}} from '../runtime.mjs';\nconst value = requireModule({});\nexport default value;\n",
                quote(&module.token)
            );
            for (index, name) in module
                .surface
                .iter()
                .filter(|name| name.as_str() != "default")
                .enumerate()
            {
                source.push_str(&format!(
                    "const v{index} = value == null ? undefined : value[{}];\nexport {{v{index} as {}}};\n",
                    quote(name), quote(name)
                ));
            }
            let path = PathBuf::from(format!("proxies/{}.mjs", digest(&proxy)));
            rewrite::validate(&path, &source)?;
            files.insert(path.clone(), source);
            cjs_proxies.insert(
                graph.file_path(file).unwrap().to_owned(),
                (proxy, module.surface.clone()),
            );
        }
    }
    for (path, payload) in payloads {
        files.insert(path, serde_json::to_string(&payload)?);
    }
    let entry_tokens: BTreeMap<_, _> = plan
        .roots
        .iter()
        .filter_map(|(root, seed)| {
            let RootId::Entry(name) = root else {
                return None;
            };
            Some((name, seed))
        })
        .map(|(name, seed)| {
            let mut evaluation: BTreeSet<_> = seed.evaluation_roots.iter().copied().collect();
            for node in &seed.binding_roots {
                match node {
                    GraphNode::Module(file) => {
                        evaluation.insert(*file);
                    }
                    GraphNode::Segment(key) => {
                        evaluation.insert(key.file_id);
                    }
                    GraphNode::Unlinked { .. } => {
                        eyre::bail!("external entry emission is unsupported")
                    }
                }
            }
            // A facade can evaluate several modules, but has no invented namespace
            // merge policy. loadEntry returns namespaces in this canonical order.
            let mut evaluation: Vec<_> = evaluation.into_iter().collect();
            evaluation.sort_by_key(|file| graph.file_path(*file));
            Ok((
                name.clone(),
                evaluation
                    .into_iter()
                    .map(|file| {
                        if modules[&file].parsed.facts.commonjs {
                            proxy_token(&modules[&file])
                        } else {
                            modules[&file].token.clone()
                        }
                    })
                    .collect::<Vec<_>>(),
            ))
        })
        .collect::<eyre::Result<_>>()?;
    let runtime = PathBuf::from("runtime.mjs");
    let formats: BTreeMap<_, _> = modules
        .values()
        .map(|module| {
            (
                module.token.clone(),
                if matches!(module.parsed.body, ModuleBody::CommonJs { .. }) {
                    "commonjs"
                } else {
                    "module"
                },
            )
        })
        .collect();
    let proxies: BTreeMap<_, _> = cjs_proxies
        .values()
        .map(|(token, _)| (token.clone(), format!("proxies/{}.mjs", digest(token))))
        .collect();
    let config = serde_json::json!({ "modules": module_pieces, "entries": entry_tokens, "formats": formats, "proxies": proxies });
    let runtime_source = format!(
        "const config = JSON.parse({});\n{}",
        quote(&config.to_string()),
        include_str!("esm_loader.mjs")
    );
    rewrite::validate(&runtime, &runtime_source)?;
    files.insert(runtime.clone(), runtime_source);
    let mut entries = BTreeMap::new();
    for root in plan.roots.keys() {
        let RootId::Entry(name) = root else {
            continue;
        };
        let path = PathBuf::from(format!("entries/{}.mjs", digest(name)));
        let source = format!(
            "import {{loadEntry}} from '../runtime.mjs';\nexport default await loadEntry({});\n",
            quote(name)
        );
        rewrite::validate(&path, &source)?;
        files.insert(path.clone(), source);
        entries.insert(name.clone(), path);
    }
    Ok(Artifacts {
        files,
        entries,
        runtime,
        modules: modules
            .into_iter()
            .map(|(file, m)| (graph.file_path(file).unwrap().to_owned(), m.token))
            .collect(),
        module_payloads,
        members,
        chunk_artifacts,
        manifest,
        cjs_proxies,
    })
}

fn dependency_edits(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    file: u32,
    modules: &BTreeMap<u32, Module<'_>>,
) -> eyre::Result<Vec<Edit>> {
    let module = &modules[&file];
    let live = |span| {
        module.parsed.segments.iter().enumerate().any(|(i, s)| {
            contains(s.span, span)
                && retained(plan, GraphNode::Segment(SegmentKey::new(file, i as u32)))
        })
    };
    let mut edits = Vec::new();
    let mut emitted = BTreeSet::new();
    for dependency in module
        .parsed
        .facts
        .dependencies
        .iter()
        .filter(|d| !d.type_only)
    {
        let is_static = matches!(
            dependency.kind,
            DependencyKind::Import | DependencyKind::ReExport
        );
        if !is_static && !live(dependency.span) {
            continue;
        }
        ensure!(
            is_static || dependency.kind == DependencyKind::Require,
            "{}: dynamic loading awaits P8 at bytes {}..{}",
            graph.file_path(file).unwrap().display(),
            dependency.span.start,
            dependency.span.end
        );
        let target = target_file(graph, file, dependency)?;
        let target_module = modules
            .get(&target)
            .ok_or_eyre("static target absent from retention")?;
        let target_specifier = quote(&if target_module.parsed.facts.commonjs
            && dependency.kind != DependencyKind::Require
        {
            proxy_token(target_module)
        } else {
            target_module.token.clone()
        });
        if dependency.kind == DependencyKind::Require {
            ensure!(
                matches!(module.parsed.body, ModuleBody::CommonJs { .. }),
                "require in ESM needs an explicit wrapper policy"
            );
            edits.push(Edit {
                span: dependency.specifier_span,
                replacement: target_specifier,
            });
            continue;
        }
        let mut replacement = format!("import {target_specifier};\n");
        if dependency.kind == DependencyKind::Import {
            for import in module
                .parsed
                .facts
                .imports
                .iter()
                .filter(|i| i.dependency == dependency.id && !i.type_only)
            {
                let used = module
                    .parsed
                    .facts
                    .references
                    .iter()
                    .any(|r| r.binding == Some(import.binding) && live(r.span))
                    || module.parsed.facts.exports.iter().any(|e| {
                        matches!(e.value, ExportValue::Binding(id) if id == import.binding)
                            && live(e.span)
                    });
                if !used {
                    continue;
                }
                let binding = module
                    .parsed
                    .facts
                    .bindings
                    .iter()
                    .find(|b| b.id == import.binding)
                    .unwrap();
                match &import.imported {
                    ImportedName::Name(name) => {
                        ensure!(
                            target_module.surface.contains(name),
                            "retained import has no emitted export"
                        );
                        replacement.push_str(&format!(
                            "import {{{} as {}}} from {target_specifier};\n",
                            quote(name),
                            binding.name
                        ));
                    }
                    ImportedName::Namespace => replacement.push_str(&format!(
                        "import * as {} from {target_specifier};\n",
                        binding.name
                    )),
                }
            }
        } else {
            let key = SegmentKey::new(
                file,
                module
                    .parsed
                    .segments
                    .iter()
                    .position(|s| contains(s.span, dependency.span))
                    .unwrap() as u32,
            );
            for name in &module.surface {
                let linked = graph.resolve_export(file, name, Namespace::Value)?;
                if !linked.route.contains(&GraphNode::Segment(key)) {
                    continue;
                }
                let export = module.parsed.facts.exports.iter().find(|e| {
                    contains(dependency.span, e.span)
                        && (e.name.as_ref() == Some(name) || e.name.is_none())
                });
                let Some(export) = export else {
                    continue;
                };
                let ExportValue::ReExport { imported, .. } = &export.value else {
                    continue;
                };
                // Equal-identity star diamonds have several checked routes but
                // may only become one explicit export declaration.
                if !emitted.insert(name.clone()) {
                    continue;
                }
                match imported {
                    ImportedName::Name(imported) => {
                        ensure!(
                            target_module.surface.contains(imported),
                            "retained re-export has no emitted export"
                        );
                        replacement.push_str(&format!(
                            "export {{{} as {}}} from {target_specifier};\n",
                            quote(imported),
                            quote(name)
                        ));
                    }
                    ImportedName::Namespace if export.name.is_some() => replacement.push_str(
                        &format!("export * as {} from {target_specifier};\n", quote(name)),
                    ),
                    ImportedName::Namespace => {
                        ensure!(
                            target_module.surface.contains(name),
                            "retained star export has no emitted export"
                        );
                        replacement.push_str(&format!(
                            "export {{{0} as {0}}} from {target_specifier};\n",
                            quote(name)
                        ));
                    }
                }
            }
        }
        edits.push(Edit {
            span: dependency.span,
            replacement,
        });
    }
    Ok(edits)
}

fn target_file(graph: &SourceGraph, file: u32, dependency: &Dependency) -> eyre::Result<u32> {
    match graph.dependency_target(file, dependency.id) {
        Some(DependencyTarget::File(path)) => {
            graph.file_id(path).ok_or_eyre("resolved target is absent")
        }
        _ => eyre::bail!(
            "{}: external/unresolved emission has no declared policy at bytes {}..{}",
            graph.file_path(file).unwrap().display(),
            dependency.span.start,
            dependency.span.end
        ),
    }
}

fn export_names(graph: &SourceGraph, plan: &ChunkPlan) -> BTreeMap<u32, BTreeSet<String>> {
    let mut names: BTreeMap<_, BTreeSet<_>> = graph
        .module_ids()
        .map(|file| {
            (
                file,
                if graph.file_facts(file).is_some_and(|facts| facts.commonjs) {
                    let mut names = BTreeSet::from(["default".to_owned()]);
                    for edge in graph.incoming(GraphNode::Module(file)).filter(|edge| {
                        !edge.type_only
                            && matches!(
                                edge.kind,
                                source_graph::EdgeKind::Import | source_graph::EdgeKind::ReExport
                            )
                            && retained(plan, edge.from)
                    }) {
                        if let Some(ImportedName::Name(name)) = &edge.imported {
                            let from = match edge.from {
                                GraphNode::Module(file) => file,
                                GraphNode::Segment(key) => key.file_id,
                                GraphNode::Unlinked { file_id, .. } => file_id,
                            };
                            let target = edge
                                .dependency
                                .and_then(|dependency| graph.dependency_target(from, dependency))
                                .and_then(|target| match target {
                                    DependencyTarget::File(path) => graph.file_id(path),
                                    _ => None,
                                });
                            if let Some(source_graph::ExportIdentity::CommonJs(target, name)) =
                                target
                                    .and_then(|target| {
                                        graph.resolve_export(target, name, Namespace::Value).ok()
                                    })
                                    .map(|linked| linked.identity)
                            {
                                if target == file {
                                    names.insert(name);
                                }
                            }
                        }
                    }
                    names
                } else {
                    graph
                        .file_facts(file)
                        .into_iter()
                        .flat_map(|f| &f.exports)
                        .filter(|e| !e.type_only)
                        .filter_map(|e| e.name.clone())
                        .collect()
                },
            )
        })
        .collect();
    loop {
        let previous = names.clone();
        for file in graph.module_ids() {
            for export in graph
                .file_facts(file)
                .into_iter()
                .flat_map(|f| &f.exports)
                .filter(|e| !e.type_only && e.name.is_none())
            {
                if let ExportValue::ReExport { dependency, .. } = export.value {
                    if let Some(DependencyTarget::File(path)) =
                        graph.dependency_target(file, dependency)
                    {
                        if let Some(target) = graph.file_id(path) {
                            names.get_mut(&file).unwrap().extend(
                                previous[&target]
                                    .iter()
                                    .filter(|n| n.as_str() != "default")
                                    .cloned(),
                            );
                        }
                    }
                }
            }
        }
        if names == previous {
            return names;
        }
    }
}

fn proxy_token(module: &Module<'_>) -> String {
    format!("{}:proxy", module.token)
}

fn retained(plan: &ChunkPlan, node: GraphNode) -> bool {
    match node {
        GraphNode::Module(file) => plan.retention.modules.contains(&file),
        GraphNode::Segment(key) => plan.retention.segments.iter().any(|s| s.key == key),
        GraphNode::Unlinked { .. } => false,
    }
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}
