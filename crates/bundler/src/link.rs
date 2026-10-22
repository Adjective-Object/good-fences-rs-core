//! Native ESM/CJS chunks.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ast_segmenter::owned::DependencyKind;
use ast_segmenter::views::{Dependency, ExportValue};
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
        self.render_to_with_maps(destinations, false)
    }

    pub(crate) fn render_to_with_maps(
        self,
        destinations: &BTreeMap<PathBuf, PathBuf>,
        source_maps: bool,
    ) -> eyre::Result<Artifacts> {
        let _span = tracing::info_span!("build.render", artifacts = self.files.len()).entered();
        let mut files = BTreeMap::new();
        for (path, code) in &self.files {
            let destination = destinations
                .get(path)
                .ok_or_eyre("missing artifact destination")?;
            let executable = crate::sourcemap::executable(destination);
            let (mut source, map) =
                code.render_with_map(path, destination, destinations, source_maps && executable)?;
            if let Some(map) = map {
                let map_path = crate::sourcemap::path(destination);
                let request = crate::assembly::relative_request(destination, &map_path, true)
                    .ok_or_eyre("source map destination cannot form a relative request")?;
                source.push_str(&format!("\n//# sourceMappingURL={request}\n"));
                ensure!(
                    files.insert(map_path, map.to_json_string()).is_none(),
                    "source map artifact collision"
                );
            }
            if executable {
                crate::rewrite::validate(destination, &source)?;
            }
            ensure!(
                files.insert(destination.clone(), source).is_none(),
                "output artifact collision"
            );
        }
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

pub(crate) struct Module<'a> {
    path: &'a std::path::Path,
    source: &'a str,
    mapping: Option<std::sync::Arc<crate::sourcemap::Source>>,
    parsed: ParsedModule,
    surface: BTreeSet<String>,
    relocations: &'a [crate::css_loading::UrlRelocation],
}

/// A physical native body identity, independent of artifact naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativeOwner {
    Evaluation(u32),
    Bindings(source_graph::SegmentKey),
}

/// Immutable final owners and typed lazy reconstruction requests. No emitted
/// text or output filenames participate in placement.
pub struct NativePlacement {
    owners: BTreeMap<source_graph::SegmentKey, NativeOwner>,
    lazy: Vec<LazyWiring>,
    bindings: BTreeMap<NativeOwner, Vec<NativeBinding>>,
    schedules: BTreeMap<NativeOwner, Vec<source_graph::SegmentKey>>,
}

impl NativePlacement {
    pub fn owners(&self) -> &BTreeMap<source_graph::SegmentKey, NativeOwner> {
        &self.owners
    }

    pub fn lazy(&self) -> &[LazyWiring] {
        &self.lazy
    }

    pub fn bindings(&self) -> &BTreeMap<NativeOwner, Vec<NativeBinding>> {
        &self.bindings
    }

    pub fn schedule(&self, owner: NativeOwner) -> &[source_graph::SegmentKey] {
        self.schedules
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBinding {
    pub symbol: (u32, ast_segmenter::owned::BindingId),
    pub identity: source_graph::ExportIdentity,
    pub owner: Option<NativeOwner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyWiring {
    pub declaration: source_graph::SegmentKey,
    pub module_candidate: u32,
    pub selector: u32,
    pub selector_segment: source_graph::SegmentKey,
    pub source: NativeOwner,
    pub selected: source_graph::ExportIdentity,
    pub destination: NativeOwner,
    pub roots: BTreeSet<RootId>,
    pub evaluation_target: u32,
    pub constructor: source_graph::ExportIdentity,
    /// Canonical constructor definition; a retained local alias may have its own owner.
    pub constructor_owner: NativeOwner,
    pub constructor_symbol: (u32, ast_segmenter::owned::BindingId),
    /// The single original options value, never an expression to duplicate.
    pub options: Option<PreservedOptionsValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedOptionsValue {
    pub segment: source_graph::SegmentKey,
    pub owner: NativeOwner,
    pub span: oxc_span::Span,
}

/// Solve native segment placement under checked transformed obligations without
/// emitting code. Results are local to this graph/plan generation; no optional
/// source-island activation is inferred. Connections consumes these same inputs
/// when emitting certified lazy wiring.
pub fn native_placement(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    options: &crate::build::OutputOptions,
) -> eyre::Result<NativePlacement> {
    ensure!(
        plan.is_current_with_options(graph, policy, options),
        "stale native placement plan"
    );
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    let relocations = BTreeMap::new();
    let mut modules = load_modules(graph, plan, sources, &relocations)?;
    let symbols = crate::symbols::Symbols::link(graph, plan)?;
    for (&file, module) in &mut modules {
        module.surface = symbols.exports[&file].keys().cloned().collect();
    }
    native::placement(
        graph,
        plan,
        &modules,
        &manifest,
        &symbols,
        options.min_chunk_bytes,
    )
}

impl<'a> Module<'a> {
    pub(crate) fn source(&self) -> &'a str {
        self.source
    }
    pub(crate) fn facts(&self) -> ast_segmenter::views::ModuleFacts<'_> {
        self.parsed.facts()
    }
    pub(crate) fn mapping(&self) -> Option<&std::sync::Arc<crate::sourcemap::Source>> {
        self.mapping.as_ref()
    }
    pub(crate) fn path(&self) -> &std::path::Path {
        self.path
    }
}

/// Emit native chunks.
pub fn esm(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Artifacts> {
    ensure!(
        plan.lazy_modules.is_current(graph, policy, &[]),
        "lazy runtime configuration requires esm_with_options"
    );
    esm_plan(graph, plan, policy, sources)?.render()
}

/// Emit native chunks with the same small-library size policy as BuildSession.
/// Resource URL preparation remains the public build's responsibility.
pub fn esm_with_options(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    options: &crate::build::OutputOptions,
) -> eyre::Result<Artifacts> {
    ensure!(
        plan.is_current_with_options(graph, policy, options),
        "stale lazy certification configuration; repartition with current options"
    );
    ensure!(
        options.synthesize_async_boundaries || plan.generated_boundaries.is_empty(),
        "generated plan requires synthesize_async_boundaries; repartition to disable"
    );
    let generated;
    let plan = if options.synthesize_async_boundaries {
        generated = crate::async_boundaries::extract(graph, plan, policy)?;
        &generated
    } else {
        plan
    };
    esm_plan_configured(
        graph,
        plan,
        policy,
        sources,
        &BTreeMap::new(),
        false,
        &BTreeSet::new(),
        &BTreeMap::new(),
        options.min_chunk_bytes,
        &BTreeMap::new(),
        Default::default(),
    )?
    .render()
}

/// Explicit reconstruction only: no activation calls or physical placement policy.
/// `None` means the island must retain ordinary native evaluation.
pub fn deferred_esm(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
) -> eyre::Result<Option<DeferredEsm>> {
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    if !plan.lazy_modules.modules().is_empty() {
        return Ok(None);
    }
    let relocations = BTreeMap::new();
    let modules = load_modules(graph, plan, sources, &relocations)?;
    let symbols = crate::symbols::Symbols::link(graph, plan)?;
    native::deferred(graph, plan, &modules, &manifest, &symbols)?
        .map(|(artifacts, initializer)| {
            Ok(DeferredEsm {
                artifacts: artifacts.render()?,
                initializer,
            })
        })
        .transpose()
}

/// A native ESM island whose root bindings are initialized by one exported guard.
/// Loading `artifacts.entries` alone never invokes `initializer`.
pub struct DeferredEsm {
    pub artifacts: Artifacts,
    pub initializer: String,
}

pub fn esm_with_execution(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts> {
    ensure!(
        plan.lazy_modules.is_current(graph, policy, &[]),
        "lazy runtime configuration requires esm_with_options"
    );
    esm_plan_configured(
        graph,
        plan,
        policy,
        sources,
        &BTreeMap::new(),
        false,
        &BTreeSet::new(),
        &BTreeMap::new(),
        0,
        &BTreeMap::new(),
        execution,
    )?
    .render()
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
        false,
        &BTreeSet::new(),
    )
}

pub(crate) fn esm_plan_prepared(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    relocations: &BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
) -> eyre::Result<Artifacts<crate::assembly::Code>> {
    esm_plan_configured(
        graph,
        plan,
        policy,
        sources,
        relocations,
        browser,
        require_defaults,
        &BTreeMap::new(),
        0,
        &BTreeMap::new(),
        Default::default(),
    )
}

/// Internal placement; an empty selection derives single-parent placements.
/// Explicit callers must exclude
/// escaped/reflective namespace observations; namespaces here support live reads
/// and identity only. Unsupported reconstruction returns native output.
#[cfg(test)]
pub(crate) fn esm_plan_activated(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    relocations: &BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
    placements: &BTreeMap<u32, u32>,
) -> eyre::Result<Artifacts<crate::assembly::Code>> {
    esm_plan_configured(
        graph,
        plan,
        policy,
        sources,
        relocations,
        false,
        &BTreeSet::new(),
        placements,
        0,
        &BTreeMap::new(),
        Default::default(),
    )
}

pub(crate) fn esm_plan_configured(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
    sources: &BTreeMap<PathBuf, String>,
    relocations: &BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
    placements: &BTreeMap<u32, u32>,
    min_chunk_bytes: usize,
    mappings: &BTreeMap<PathBuf, std::sync::Arc<crate::sourcemap::Source>>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts<crate::assembly::Code>> {
    let _span = tracing::info_span!("build.link").entered();
    let manifest = diagnostic_manifest(graph, plan, policy)?;
    graph.validate_links()?;
    let mut modules = load_modules(graph, plan, sources, relocations)?;
    for module in modules.values_mut() {
        module.mapping = mappings.get(module.path).cloned();
    }
    let symbols = crate::symbols::Symbols::link(graph, plan)?;
    for (&file, module) in &mut modules {
        module.surface = symbols.exports[&file].keys().cloned().collect();
    }
    native::emit(
        graph,
        plan,
        &modules,
        &manifest,
        &symbols,
        browser,
        require_defaults,
        placements,
        min_chunk_bytes,
        execution,
    )
}

fn load_modules<'a>(
    graph: &'a SourceGraph,
    plan: &ChunkPlan,
    sources: &'a BTreeMap<PathBuf, String>,
    relocations: &'a BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
) -> eyre::Result<BTreeMap<u32, Module<'a>>> {
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
                path,
                source,
                mapping: None,
                parsed,
                surface: BTreeSet::new(),
                relocations: relocations.get(path).map(Vec::as_slice).unwrap_or_default(),
            },
        );
    }
    Ok(modules)
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
                .facts()
                .top_level_await()
                .first()
                .map(|span| (file, (file, span)))
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
            for dependency in module.parsed.facts().dependencies().iter().filter(|d| {
                !d.type_only()
                    && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
            }) {
                if matches!(
                    graph.dependency_target(file, dependency.id()),
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
            .facts()
            .dependencies()
            .iter()
            .filter(|d| !d.type_only() && d.kind() == DependencyKind::Require)
        {
            for target in target_files(graph, file, dependency)? {
                if let Some(&(origin, span)) = asynchronous.get(&target) {
                    eyre::bail!(
                    "{}: require of async ESM {} at bytes {}..{} (top-level await in {} at bytes {}..{})",
                    graph.file_path(file).unwrap().display(),
                    graph.file_path(target).unwrap().display(),
                    dependency.span().start,
                    dependency.span().end,
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
    dependency: Dependency<'_>,
) -> eyre::Result<u32> {
    match graph.dependency_target(file, dependency.id()) {
        Some(DependencyTarget::File(path)) => {
            graph.file_id(path).ok_or_eyre("resolved target is absent")
        }
        _ => eyre::bail!(
            "{}: external/unresolved emission has no declared policy at bytes {}..{}",
            graph.file_path(file).unwrap().display(),
            dependency.span().start,
            dependency.span().end
        ),
    }
}

pub(super) fn target_files(
    graph: &SourceGraph,
    file: u32,
    dependency: Dependency<'_>,
) -> eyre::Result<Vec<u32>> {
    if matches!(
        graph.dependency_target(file, dependency.id()),
        Some(DependencyTarget::External(_))
    ) {
        Ok(vec![])
    } else if let Some(DependencyTarget::Context(candidates)) =
        graph.dependency_target(file, dependency.id())
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
        .flat_map(|facts| facts.dependencies())
        .any(|dependency| {
            !dependency.type_only()
                && matches!(
                    graph.dependency_target(file, dependency.id()),
                    Some(DependencyTarget::External(_))
                )
        })
}

pub(super) fn export_names(
    graph: &SourceGraph,
    plan: &ChunkPlan,
) -> BTreeMap<u32, BTreeSet<String>> {
    namespace_export_names(graph, |file| crate::cjs::planned_names(graph, plan, file))
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
                if graph.file_facts(file).is_some_and(|facts| facts.commonjs()) {
                    commonjs(file)
                } else {
                    graph
                        .file_facts(file)
                        .into_iter()
                        .flat_map(|f| f.exports())
                        .filter(|e| !e.type_only())
                        .filter_map(|e| e.name().map(str::to_owned))
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
                .flat_map(|f| f.exports())
                .filter(|e| !e.type_only() && e.name().is_none())
            {
                if let ExportValue::ReExport { dependency, .. } = export.value() {
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
