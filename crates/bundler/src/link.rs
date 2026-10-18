//! Retained original lexical modules linked as ordinary native JavaScript files.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ast_segmenter::owned::{Dependency, DependencyKind, ExportValue, ImportedName};
use ast_segmenter::{ModuleBody, ParsedModule};
use eyre::{ensure, OptionExt, WrapErr};
use oxc_span::Span;
use sha2::{Digest, Sha256};
use source_graph::{DependencyTarget, GraphNode, Namespace, SegmentKey, SourceGraph};

use crate::chunks::{AsyncSiteId, ChunkId, ChunkPlan, NodeId, RootId};
use crate::manifest::{diagnostic_manifest, DiagnosticManifest};
use crate::policy::EffectPolicy;
use crate::rewrite::{self, Edit};

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

/// Owned staging result; no filesystem writes. Publish the complete relative
/// map together and execute/import `entries` with Node's ordinary loader.
/// Exact logical chunks are diagnostic: physical files preserve original
/// lexical modules and can therefore belong to several logical chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifacts {
    pub files: BTreeMap<PathBuf, String>,
    /// Direct native files; multi-module low-level roots are evaluation-only.
    pub entries: BTreeMap<String, PathBuf>,
    /// Source identities to canonical executable relative paths.
    pub modules: BTreeMap<PathBuf, PathBuf>,
    /// Logical members to physical files; this mapping is deliberately many-to-one.
    pub members: BTreeMap<NodeId, PathBuf>,
    /// Physical projections of logical chunks, which may overlap each other.
    pub chunk_artifacts: BTreeMap<ChunkId, BTreeSet<PathBuf>>,
    pub manifest: DiagnosticManifest,
    /// CJS identity -> native ESM proxy file and its static snapshot surface.
    pub cjs_proxies: BTreeMap<PathBuf, (PathBuf, BTreeSet<String>)>,
    /// Files whose default export is the conservative runtime-key CJS bridge.
    pub cjs_namespaces: BTreeMap<PathBuf, PathBuf>,
    /// Reached source occurrences -> canonical executable target files.
    pub async_facades: BTreeMap<AsyncSiteId, PathBuf>,
    /// ESM modules with source TLA or a transitive static async dependency.
    /// Native ESM owns SCC evaluation jobs, suspension and rejection propagation.
    pub async_modules: BTreeSet<PathBuf>,
}

struct Module<'a> {
    source: &'a str,
    parsed: ParsedModule,
    path: PathBuf,
    surface: BTreeSet<String>,
}

/// Native ESM owns live bindings/TDZ and native CJS owns wrappers/cache.
/// Retained statements remain together in their original lexical environment.
pub fn esm(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Artifacts> {
    esm_with_source_checker(graph, plan, policy, sources, |file, source| {
        crate::emit::checked_source(graph, file, source)
    })
}

pub(crate) fn esm_with_source_checker(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    mut checked_source: impl FnMut(u32, &str) -> eyre::Result<ParsedModule>,
) -> eyre::Result<Artifacts> {
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    let mut modules = BTreeMap::new();
    for &file in &plan.retention.modules {
        let path = graph.file_path(file).ok_or_eyre("missing linked module")?;
        let source = sources
            .get(path)
            .ok_or_else(|| eyre::eyre!("{}: missing linked source text", path.display()))?;
        let parsed = checked_source(file, source)?;
        let identity = path
            .to_str()
            .ok_or_eyre("emission module paths must be UTF-8")?;
        modules.insert(
            file,
            Module {
                source,
                path: PathBuf::from(format!(
                    "modules/{}.{}",
                    digest(identity),
                    if parsed.facts.commonjs { "cjs" } else { "mjs" }
                )),
                parsed,
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
    let async_modules = async_modules(graph, &modules)?;
    let mut files = BTreeMap::new();
    let mut members = BTreeMap::new();
    let mut chunk_artifacts = BTreeMap::new();
    for (chunk, nodes) in &plan.chunks {
        let mut paths = BTreeSet::new();
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
            let path = match node {
                NodeId::Module(path) | NodeId::Segment { module: path, .. } => modules
                    [&graph.file_id(path).ok_or_eyre("missing member module")?]
                    .path
                    .clone(),
                NodeId::CommonJsProxy { module, .. } => {
                    proxy_path(&modules[&graph.file_id(module).ok_or_eyre("missing proxy module")?])
                }
                NodeId::CommonJsNamespace(module) => namespace_path(
                    &modules[&graph
                        .file_id(module)
                        .ok_or_eyre("missing namespace module")?],
                ),
                NodeId::External { .. } => unreachable!(),
            };
            paths.insert(path.clone());
            members.insert(node.clone(), path);
        }
        chunk_artifacts.insert(chunk.clone(), paths);
    }
    let mut cjs_proxies = BTreeMap::new();
    let mut cjs_namespaces = BTreeMap::new();
    for (&file, module) in &modules {
        let edits = dependency_edits(graph, plan, file, &modules)?;
        let source = if matches!(module.parsed.body, ModuleBody::CommonJs { .. }) {
            let source = rewrite::apply(module.source, &edits)?;
            let source = rewrite::apply(&source, &rewrite::source_hint_edits(&source))?;
            rewrite::validate(&module.path, &source)?;
            source
        } else {
            crate::emit::project(
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
        files.insert(module.path.clone(), source);
        let module_path = graph.file_path(file).unwrap().to_owned();
        let initialize = NodeId::CommonJsProxy {
            module: module_path.clone(),
            name: None,
        };
        if module.parsed.facts.commonjs && members.contains_key(&initialize) {
            let proxy = proxy_path(module);
            let mut source = format!(
                "import {{createRequire}} from 'node:module';\nconst value = createRequire(import.meta.url)({});\nexport default value;\n",
                specifier(&module.path)
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
            rewrite::validate(&proxy, &source)?;
            files.insert(proxy.clone(), source);
            cjs_proxies.insert(
                graph.file_path(file).unwrap().to_owned(),
                (proxy, module.surface.clone()),
            );
        }
        let namespace_node = NodeId::CommonJsNamespace(module_path.clone());
        if let Some(path) = members.get(&namespace_node) {
            let source = format!(
                "import value from {};\n{}\nexport default commonjsNamespace(value);\n",
                specifier(&proxy_path(module)),
                include_str!("commonjs_namespace.mjs")
            );
            rewrite::validate(path, &source)?;
            files.insert(path.clone(), source);
            cjs_namespaces.insert(module_path, path.clone());
        }
    }
    let mut entries = BTreeMap::new();
    for (name, seed) in plan.roots.iter().filter_map(|(root, seed)| {
        let RootId::Entry(name) = root else {
            return None;
        };
        Some((name, seed))
    }) {
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
        let mut evaluation: Vec<_> = evaluation.into_iter().collect();
        evaluation.sort_by_key(|file| graph.file_path(*file));
        let path = if evaluation.len() == 1 {
            modules[&evaluation[0]].path.clone()
        } else {
            // Multi-module low-level roots have evaluation demand but no
            // invented namespace merge. Native static imports preserve order.
            let path = PathBuf::from(format!("modules/entry-{}.mjs", digest(name)));
            let source: String = evaluation
                .iter()
                .map(|file| format!("import {};\n", specifier(&modules[file].path)))
                .collect();
            rewrite::validate(&path, &source)?;
            files.insert(path.clone(), source);
            path
        };
        entries.insert(name.clone(), path);
    }
    let async_facades = manifest
        .async_facades
        .iter()
        .map(|(site, facade)| {
            let NodeId::Module(path) = &facade.target else {
                eyre::bail!("async facade target must be a linked module");
            };
            let file = graph.file_id(path).ok_or_eyre("missing async target")?;
            let module = &modules[&file];
            Ok((
                site.clone(),
                if module.parsed.facts.commonjs {
                    namespace_path(module)
                } else {
                    module.path.clone()
                },
            ))
        })
        .collect::<eyre::Result<_>>()?;
    Ok(Artifacts {
        files,
        entries,
        modules: modules
            .into_iter()
            .map(|(file, m)| (graph.file_path(file).unwrap().to_owned(), m.path))
            .collect(),
        members,
        chunk_artifacts,
        manifest,
        cjs_proxies,
        cjs_namespaces,
        async_facades,
        async_modules,
    })
}

fn async_modules(
    graph: &SourceGraph,
    modules: &BTreeMap<u32, Module<'_>>,
) -> eyre::Result<BTreeSet<PathBuf>> {
    let mut asynchronous: BTreeMap<_, _> = modules
        .iter()
        .filter_map(|(&file, module)| {
            module
                .parsed
                .facts
                .top_level_await
                .first()
                .map(|&span| (file, (file, span)))
        })
        .collect();
    // Static dependency closure also propagates async status around SCCs.
    // Dynamic imports and deferred requires never make their consumer async.
    loop {
        let mut changed = false;
        for (&file, module) in modules {
            if asynchronous.contains_key(&file) {
                continue;
            }
            for dependency in module.parsed.facts.dependencies.iter().filter(|d| {
                !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            }) {
                let target = target_file(graph, file, dependency)?;
                if let Some(&origin) = asynchronous.get(&target) {
                    asynchronous.insert(file, origin);
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for (&file, module) in modules {
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .filter(|d| !d.type_only && d.kind == DependencyKind::Require)
        {
            let target = target_file(graph, file, dependency)?;
            if let Some(&(origin, span)) = asynchronous.get(&target) {
                eyre::bail!(
                    "{}: require of async ESM {} at bytes {}..{} (top-level await in {} at bytes {}..{})",
                    graph.file_path(file).unwrap().display(),
                    graph.file_path(target).unwrap().display(),
                    dependency.span.start,
                    dependency.span.end,
                    graph.file_path(origin).unwrap().display(),
                    span.start,
                    span.end
                );
            }
        }
    }
    Ok(asynchronous
        .keys()
        .map(|&file| graph.file_path(file).unwrap().to_owned())
        .collect())
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
        let target = target_file(graph, file, dependency)?;
        let target_module = modules
            .get(&target)
            .ok_or_eyre("static target absent from retention")?;
        if dependency.kind == DependencyKind::DynamicImport {
            let specifier = if target_module.parsed.facts.commonjs {
                namespace_specifier(target_module)
            } else {
                specifier(&target_module.path)
            };
            edits.push(Edit {
                span: dependency.span,
                replacement: if target_module.parsed.facts.commonjs {
                    format!("import({specifier}).then(namespace => namespace.default)")
                } else {
                    format!("import({specifier})")
                },
            });
            continue;
        }
        let target_specifier = if dependency.kind == DependencyKind::Require {
            specifier(&target_module.path)
        } else {
            let path = if target_module.parsed.facts.commonjs {
                proxy_path(target_module)
            } else {
                target_module.path.clone()
            };
            specifier(&path)
        };
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
                    ImportedName::Namespace => {
                        if target_module.parsed.facts.commonjs {
                            replacement.push_str(&format!(
                                "import {} from {};\n",
                                binding.name,
                                namespace_specifier(target_module)
                            ));
                        } else {
                            replacement.push_str(&format!(
                                "import * as {} from {target_specifier};\n",
                                binding.name
                            ));
                        }
                    }
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
                    ImportedName::Namespace if export.name.is_some() => {
                        if target_module.parsed.facts.commonjs {
                            replacement.push_str(&format!(
                                "export {{default as {}}} from {};\n",
                                quote(name),
                                namespace_specifier(target_module)
                            ));
                        } else {
                            replacement.push_str(&format!(
                                "export * as {} from {target_specifier};\n",
                                quote(name)
                            ));
                        }
                    }
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
                    crate::cjs::names(graph, &plan.retention, file)
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

fn proxy_path(module: &Module<'_>) -> PathBuf {
    module.path.with_extension("proxy.mjs")
}

fn namespace_path(module: &Module<'_>) -> PathBuf {
    module.path.with_extension("namespace.mjs")
}

fn namespace_specifier(module: &Module<'_>) -> String {
    specifier(&namespace_path(module))
}

fn specifier(path: &std::path::Path) -> String {
    quote(&format!(
        "./{}",
        path.file_name().unwrap().to_str().unwrap()
    ))
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
