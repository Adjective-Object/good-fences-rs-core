//! Native ESM/CJS chunks.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ast_segmenter::owned::{Dependency, DependencyKind, ExportValue};
use ast_segmenter::ParsedModule;
use eyre::{ensure, OptionExt};
use oxc_span::Span;
use sha2::{Digest, Sha256};
use source_graph::{DependencyTarget, GraphNode, SegmentKey, SourceGraph};

use crate::chunks::{AsyncSiteId, ChunkId, ChunkPlan, NodeId, RootId};
use crate::manifest::{diagnostic_manifest, DiagnosticManifest};
use crate::policy::EffectPolicy;
#[path = "native.rs"]
mod native;

pub(crate) const RUNTIME_PATH: &str = "native/runtime.cjs";

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

/// Emitter selected for this build, not inferred from artifact names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Native,
}

impl Backend {
    pub fn layout(self) -> &'static str {
        match self {
            Self::Native => "native-chunks-v1",
        }
    }
}

/// Owned staging result; no filesystem writes. Publish the complete relative
/// map together and execute/import `entries` with Node's ordinary loader.
/// Each retained node has one actual executable chunk owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifacts<C = String> {
    pub backend: Backend,
    pub files: BTreeMap<PathBuf, C>,
    /// Non-executable, byte-preserving resources keyed by output-relative paths.
    pub resources: BTreeMap<PathBuf, Vec<u8>>,
    /// Input CSS roots to owned output-relative CSS paths, never public URLs.
    /// Independent of loading strategy and executable entry aliases.
    pub stylesheets: BTreeMap<PathBuf, PathBuf>,
    /// Direct native files; multi-module low-level roots are evaluation-only.
    pub entries: BTreeMap<String, PathBuf>,
    /// Independently executable source namespaces to relative paths.
    /// Merged private native sources have owners in `members`, not facade paths.
    pub modules: BTreeMap<PathBuf, PathBuf>,
    /// Actual executable chunk ownership.
    pub members: BTreeMap<NodeId, PathBuf>,
    /// Actual executable files implementing each logical chunk.
    pub chunk_artifacts: BTreeMap<ChunkId, BTreeSet<PathBuf>>,
    pub manifest: DiagnosticManifest,
    /// CJS identity -> native ESM proxy file and its static snapshot surface.
    pub cjs_proxies: BTreeMap<PathBuf, (PathBuf, BTreeSet<String>)>,
    /// Files whose default export is the conservative runtime-key CJS bridge.
    pub cjs_namespaces: BTreeMap<PathBuf, PathBuf>,
    /// Reached source occurrences -> canonical executable target files.
    pub async_facades: BTreeMap<AsyncSiteId, PathBuf>,
    /// ESM modules with source TLA or a transitive static async dependency.
    /// Native body chunks own SCC jobs and rejection.
    pub async_modules: BTreeSet<PathBuf>,
}

impl Artifacts<crate::assembly::Code> {
    pub(crate) fn render(self) -> eyre::Result<Artifacts> {
        let destinations = self
            .files
            .keys()
            .map(|path| (path.clone(), path.clone()))
            .collect();
        self.render_to(&destinations)
    }

    pub(crate) fn render_to(
        self,
        destinations: &BTreeMap<PathBuf, PathBuf>,
    ) -> eyre::Result<Artifacts> {
        let _span = tracing::info_span!("build.render", artifacts = self.files.len()).entered();
        let files = self
            .files
            .iter()
            .map(|(path, code)| {
                let destination = destinations
                    .get(path)
                    .ok_or_eyre("missing artifact destination")?;
                let source = code.render(path, destination, destinations)?;
                if destination
                    .extension()
                    .is_some_and(|ext| matches!(ext.to_str(), Some("mjs" | "cjs" | "js")))
                {
                    crate::rewrite::validate(destination, &source)?;
                }
                Ok((destination.clone(), source))
            })
            .collect::<eyre::Result<_>>()?;
        Ok(Artifacts {
            backend: self.backend,
            files,
            resources: self.resources,
            stylesheets: self.stylesheets,
            entries: self.entries,
            modules: self.modules,
            members: self.members,
            chunk_artifacts: self.chunk_artifacts,
            manifest: self.manifest,
            cjs_proxies: self.cjs_proxies,
            cjs_namespaces: self.cjs_namespaces,
            async_facades: self.async_facades,
            async_modules: self.async_modules,
        })
    }
}

impl Artifacts {
    /// All owned output bytes, including executable text, metadata and resources.
    pub fn file_bytes(&self) -> impl Iterator<Item = (&PathBuf, &[u8])> {
        self.files
            .iter()
            .map(|(path, text)| (path, text.as_bytes()))
            .chain(
                self.resources
                    .iter()
                    .map(|(path, bytes)| (path, bytes.as_slice())),
            )
    }
}

struct Module<'a> {
    source: &'a str,
    parsed: ParsedModule,
    surface: BTreeSet<String>,
    relocations: &'a [crate::css_loading::UrlRelocation],
}

/// Emit native chunks.
pub fn esm(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Artifacts> {
    esm_plan(graph, plan, policy, sources)?.render()
}

/// Emit with an explicit bounded worker count. Output bytes and diagnostics are
/// independent of the count; workers never publish or finalize artifacts.
pub fn esm_with_execution(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts> {
    esm_plan_prepared(graph, plan, policy, sources, &BTreeMap::new(), execution)?.render()
}

pub(crate) fn esm_plan(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Artifacts<crate::assembly::Code>> {
    esm_plan_prepared(
        graph,
        plan,
        policy,
        sources,
        &BTreeMap::new(),
        Default::default(),
    )
}

pub(crate) fn esm_plan_prepared(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    relocations: &BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts<crate::assembly::Code>> {
    let _span = tracing::info_span!("build.link").entered();
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    let mut modules = BTreeMap::new();
    for &file in &plan.retention.modules {
        let path = graph.file_path(file).ok_or_eyre("missing linked module")?;
        let source = sources
            .get(path)
            .ok_or_else(|| eyre::eyre!("{}: missing linked source text", path.display()))?;
        let parsed = crate::emit::checked_source(graph, file, source)?;
        path.to_str()
            .ok_or_eyre("emission module paths must be UTF-8")?;
        modules.insert(
            file,
            Module {
                source,
                parsed,
                surface: BTreeSet::new(),
                relocations: relocations.get(path).map(Vec::as_slice).unwrap_or_default(),
            },
        );
    }
    let symbols = tracing::info_span!("build.symbols")
        .in_scope(|| crate::symbols::Symbols::link(graph, plan))?;
    for (&file, module) in &mut modules {
        module.surface = symbols.exports[&file].keys().cloned().collect();
    }
    native::emit(graph, plan, &modules, &manifest, &symbols, execution)
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
                if matches!(
                    graph.dependency_target(file, dependency.id),
                    Some(DependencyTarget::External(_))
                ) {
                    continue;
                }
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
            for target in target_files(graph, file, dependency)? {
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
    }
    Ok(asynchronous
        .keys()
        .map(|&file| graph.file_path(file).unwrap().to_owned())
        .collect())
}

pub(super) fn target_file(
    graph: &SourceGraph,
    file: u32,
    dependency: &Dependency,
) -> eyre::Result<u32> {
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

pub(super) fn target_files(
    graph: &SourceGraph,
    file: u32,
    dependency: &Dependency,
) -> eyre::Result<Vec<u32>> {
    if matches!(
        graph.dependency_target(file, dependency.id),
        Some(DependencyTarget::External(_))
    ) {
        Ok(vec![])
    } else if let Some(DependencyTarget::Context(candidates)) =
        graph.dependency_target(file, dependency.id)
    {
        candidates
            .values()
            .map(|path| graph.file_id(path).ok_or_eyre("context target is absent"))
            .collect::<eyre::Result<BTreeSet<_>>>()
            .map(|targets| targets.into_iter().collect())
    } else {
        Ok(vec![target_file(graph, file, dependency)?])
    }
}

pub(super) fn has_externals(graph: &SourceGraph, file: u32) -> bool {
    graph
        .file_facts(file)
        .into_iter()
        .flat_map(|facts| &facts.dependencies)
        .any(|dependency| {
            !dependency.type_only
                && matches!(
                    graph.dependency_target(file, dependency.id),
                    Some(DependencyTarget::External(_))
                )
        })
}

pub(super) fn export_names(
    graph: &SourceGraph,
    plan: &ChunkPlan,
) -> BTreeMap<u32, BTreeSet<String>> {
    namespace_export_names(graph, |file| {
        crate::cjs::names(graph, &plan.retention, file)
    })
}

pub(super) fn namespace_export_names(
    graph: &SourceGraph,
    commonjs: impl Fn(u32) -> BTreeSet<String>,
) -> BTreeMap<u32, BTreeSet<String>> {
    let mut names: BTreeMap<_, BTreeSet<_>> = graph
        .module_ids()
        .map(|file| {
            (
                file,
                if graph.file_facts(file).is_some_and(|facts| facts.commonjs) {
                    commonjs(file)
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

pub(super) fn retained(plan: &ChunkPlan, node: GraphNode) -> bool {
    match node {
        GraphNode::Module(file) => plan.retention.modules.contains(&file),
        GraphNode::Segment(key) => plan.retention.segments.iter().any(|s| s.key == key),
        GraphNode::Unlinked { .. } => true,
    }
}

pub(super) fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}
