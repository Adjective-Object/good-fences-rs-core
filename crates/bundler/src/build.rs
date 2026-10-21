//! Incremental orchestration over the shared parse, resolver, graph and linker.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use ast_segmenter::{
    fingerprint::Fingerprint, owned::DependencyKind, ModuleFormat, PreparedDeclarations,
};
use eyre::{ensure, WrapErr};
use import_resolver::{
    node_resolver::caching::NodeModulesResolverOptions,
    resolve::TargetEnv,
    structural::{DependencyResolver, ResolutionMode, ResolvedDependency, StructuralResolver},
};
use segment_cache::{CacheMode, CacheRecord, CacheStatus, CacheWrite, ParserConfig};
use sha2::{Digest, Sha256};
use source_graph::{DependencyTarget, GraphNode, OwnedSourceFileInput, SourceGraph};

use crate::{
    chunks::{self, ChunkPlan, RootId, RootSeed},
    link::{self, Artifacts},
    policy::EffectPolicy,
};

#[path = "entry_outputs.rs"]
mod entry_outputs;
mod prepare;

pub use crate::css_selection::CssModules;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// Relative output path; None preserves the source path within the project.
    pub output_name: Option<PathBuf>,
    /// Retain the public namespace, rather than evaluation alone.
    pub namespace: bool,
}

#[derive(Clone)]
pub struct BuildOptions {
    /// Explicit bounded workers; defaults to inline serial execution.
    pub execution: crate::execution::ExecutionOptions,
    /// Logical entry IDs mapped to source entries and optional output names.
    pub entries: BTreeMap<String, Entry>,
    pub resolver: NodeModulesResolverOptions,
    pub externals: BTreeSet<String>,
    pub effects: EffectPolicy,
    /// JavaScript and target-selected host contracts, followed by exact user overrides.
    pub declarations: ecco::DeclarationConfig,
    pub cache: CacheMode,
    pub output: OutputOptions,
    pub css: CssOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CssOptions {
    pub load: CssLoad,
    pub css_modules: CssModules,
}

/// Built-in stylesheet loading policies; custom strategies are not public API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CssLoad {
    #[default]
    CallerLoaded,
    AutoMount,
}

impl CssLoad {
    fn strategy(self) -> &'static dyn crate::css_loading::CssLoadingStrategy {
        match self {
            Self::CallerLoaded => &crate::css_loading::CallerLoaded,
            Self::AutoMount => &crate::css_loading::AutoMount,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputOptions {
    /// Public resource directory URL, relative to the output root by default.
    pub base_url: String,
    /// Approximate retained UTF-8 bytes per small native library; zero disables clustering.
    pub min_chunk_bytes: usize,
    /// Introduce suspension/load failures at supported private async-body references.
    pub synthesize_async_boundaries: bool,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            base_url: "./".into(),
            min_chunk_bytes: 0,
            synthesize_async_boundaries: false,
        }
    }
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            execution: crate::execution::ExecutionOptions::default(),
            entries: BTreeMap::new(),
            resolver: NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            externals: BTreeSet::new(),
            effects: EffectPolicy::default(),
            declarations: ecco::DeclarationConfig::default(),
            cache: CacheMode::ReadWrite,
            output: OutputOptions::default(),
            css: CssOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprints {
    pub parse: Fingerprint,
    pub resolution: Fingerprint,
    pub effects: [u8; 32],
    pub roots: Fingerprint,
    pub runtime: Fingerprint,
    pub output: Fingerprint,
}

#[derive(Debug, Default, Clone)]
pub struct Counters {
    /// Actual source extraction cache misses (emission checks are separate).
    pub source_parses: usize,
    /// Actual declaration serializations/hashes, including worker-local calls.
    pub declaration_fingerprints: usize,
    pub memory_hits: usize,
    pub disk_hits: usize,
    pub resolutions: usize,
    pub emissions: usize,
    /// Source re-extraction outside cache misses; emission must leave this zero.
    pub source_reparses: usize,
    /// Generated-fragment parses excluding final validation; must remain zero.
    pub fragment_reparses: usize,
    /// One per final executable artifact in debug builds, zero in release.
    pub final_validations: usize,
    /// Total emission parses, retained for compatibility with existing clients.
    pub output_parses: usize,
    pub elapsed: Duration,
}

impl Counters {
    fn record_parses(&mut self, before: crate::rewrite::ParseCounts) {
        let counts = crate::rewrite::parse_counts().difference(before);
        self.source_reparses = counts.source - self.source_parses;
        self.final_validations = counts.validation;
        self.fragment_reparses = counts.output - self.final_validations;
        self.declaration_fingerprints = counts.declarations;
        self.output_parses = self.source_reparses + self.fragment_reparses + self.final_validations;
    }
}

pub struct BuildProduct {
    /// Monotonic successful snapshot identity within this session. Graph-local
    /// handles/plans must never be transferred between product generations.
    pub generation: u64,
    pub graph: SourceGraph,
    pub plan: ChunkPlan,
    pub artifacts: Artifacts,
    pub fingerprints: Fingerprints,
}

pub struct BuildResult {
    pub product: Rc<BuildProduct>,
    pub counters: Counters,
    /// Changed/deleted modules and transitive users from the prior shared graph.
    pub invalidated: BTreeSet<PathBuf>,
    /// Nonfatal cache-write failures and effect-order cycles. Plan warnings are
    /// repeated on no-op reuse so diagnostics do not depend on cache state.
    pub warnings: Vec<String>,
}

struct Source {
    css_identity: Option<PathBuf>,
    input: Vec<u8>,
    text: String,
    record: CacheRecord,
}

/// One caller owns a session (`&mut self`); independent sessions share sidecars
/// safely. Failed builds never replace the last successful product.
pub struct BuildSession {
    root: PathBuf,
    sources: BTreeMap<PathBuf, Source>,
    resolutions: BTreeMap<PathBuf, HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>>,
    resolution_key: Option<Fingerprint>,
    declarations: Option<(
        ecco::DeclarationConfig,
        ecco::Environment,
        PreparedDeclarations,
    )>,
    pending_invalidated: BTreeSet<PathBuf>,
    last: Option<Rc<BuildProduct>>,
}

// Publication ownership/cleanup uses this integrity hash, not the internal keys.
pub(crate) fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl BuildSession {
    pub fn new(root: impl AsRef<Path>) -> eyre::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        ensure!(root.is_dir(), "build root is not a directory");
        Ok(Self {
            root,
            sources: BTreeMap::new(),
            resolutions: BTreeMap::new(),
            resolution_key: None,
            declarations: None,
            pending_invalidated: BTreeSet::new(),
            last: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn build(&mut self, options: &BuildOptions) -> eyre::Result<BuildResult> {
        let _span = tracing::info_span!(
            "build",
            entries = options.entries.len(),
            workers = options.execution.workers().get()
        )
        .entered();
        let start = Instant::now();
        ensure!(
            !options.entries.is_empty(),
            "build requires at least one entry"
        );
        ensure!(
            !options.resolver.preserve_symlinks,
            "build supports canonical module identities only"
        );
        let mut counters = Counters::default();
        let before_parses = crate::rewrite::parse_counts();
        let mut warnings = Vec::new();
        let public_base = crate::output_url::PublicBase::new(&options.output.base_url)?;
        options.css.css_modules.validate()?;
        let declaration_span = tracing::info_span!("build.declarations").entered();
        let environment = match options.resolver.target_env {
            TargetEnv::Node => ecco::Environment::Node,
            TargetEnv::Browser => ecco::Environment::Browser,
        };
        if self
            .declarations
            .as_ref()
            .is_none_or(|(config, host, _)| config != &options.declarations || *host != environment)
        {
            let prepared =
                crate::effect_analysis::prepare_declarations(&options.declarations, environment)?;
            self.declarations = Some((options.declarations.clone(), environment, prepared));
        }
        let declarations = self
            .declarations
            .as_ref()
            .expect("prepared declarations")
            .2
            .clone();
        let declarations_key = declarations.fingerprint();
        drop(declaration_span);
        let inventory = inventory(&self.root)?;
        let mut styles = crate::css::Styles::new(
            &self.root,
            &inventory,
            &public_base,
            &options.css.css_modules,
        );
        let mut aliases: Vec<_> = options.resolver.alias.iter().collect();
        aliases.sort();
        let resolution_key = Fingerprint::of(
            format!(
                "resolution-v1:{inventory:?}:{:?}:{aliases:?}:{:?}:{:?}:{:?}:{}:{:?}",
                options.resolver.target_env,
                options.resolver.extensions,
                options.resolver.export_conditions,
                options.externals,
                options.resolver.ignore_node_modules,
                options.resolver.preserve_symlinks
            )
            .as_bytes(),
        );
        let resolution_changed = self.resolution_key != Some(resolution_key);
        if resolution_changed {
            self.resolutions.clear();
            self.pending_invalidated
                .extend(self.sources.keys().cloned());
            self.resolution_key = Some(resolution_key);
        }
        let external_sources = inventory.has_external_sources(&self.root);
        let resolver = resolver::BuildResolver::new(&self.root, options, external_sources);
        let mut queue = VecDeque::new();
        let mut entries = BTreeMap::new();
        for (name, entry) in &options.entries {
            ensure!(!name.is_empty(), "empty entry name");
            let path = self
                .root
                .join(&entry.path)
                .canonicalize()
                .wrap_err_with(|| format!("entry {}: {}", name, entry.path.display()))?;
            ensure!(
                inventory.contains(&path),
                "entry outside reachable build inventory: {}",
                path.display()
            );
            entries.insert(name.clone(), (path.clone(), entry.namespace));
            queue.push_back(path);
        }
        let mut scheduled = BTreeSet::new();
        let mut resources = BTreeSet::new();
        let initial: BTreeSet<_> = queue.into_iter().collect();
        let mut jobs = prepare::enqueue(initial, &mut scheduled, &mut resources, options);
        let mut inputs = Vec::new();
        let mut sources = BTreeMap::new();
        let mut loaded_sources = BTreeMap::new();
        let previous = &self.sources;
        let mut relocations = BTreeMap::new();
        let mut require_defaults = BTreeSet::new();
        let mut changed = BTreeSet::new();
        let mut errors = BTreeMap::new();
        let root = &self.root;
        #[cfg(test)]
        let observer = prepare::AFTER_PREPARE.with(|observer| observer.borrow().clone());
        let read = |path: PathBuf| {
            let result = prepare::read(
                root,
                &path,
                &inventory,
                previous.get(&path),
                options,
                &declarations,
                &public_base,
            );
            #[cfg(test)]
            if let Some(observer) = &observer {
                observer(&path);
            }
            (path, result)
        };
        let mut integrate = |(path, prepared): (PathBuf, eyre::Result<prepare::Prepared>)| {
            let _span = tracing::info_span!("build.integrate").entered();
            let mut discovered = BTreeSet::new();
            let result = (|| -> eyre::Result<()> {
                let kind = crate::resource::kind(&path, &options.css.css_modules);
                if matches!(kind, crate::resource::ResourceKind::Json | crate::resource::ResourceKind::Asset)
                    || (kind == crate::resource::ResourceKind::Stylesheet { module: false }
                        && options.css.load == CssLoad::CallerLoaded)
                {
                    require_defaults.insert(path.clone());
                }
                let wrapper_format = match options.resolver.target_env {
                    TargetEnv::Browser => ModuleFormat::Esm,
                    TargetEnv::Node => ModuleFormat::CommonJs,
                };
                let mut css_identity = None;
                let loaded = match prepared? {
                    prepare::Prepared::Script(loaded) => loaded,
                    prepare::Prepared::Resource { input, format } => {
                        let prepared_css =
                            if matches!(kind, crate::resource::ResourceKind::Stylesheet { .. }) {
                                styles.discover(&path, &input, &resolver)?;
                                Some(options.css.load.strategy().generate_module(
                                    &crate::css_loading::CssModuleInput {
                                        stylesheet: &path,
                                        public_url: &styles.public_url(&path)?,
                                        name_mappings:
                                            styles.name_mappings(&path).unwrap_or(&BTreeMap::new()),
                                        is_module: styles.name_mappings(&path).is_some(),
                                        evaluation_dependencies:
                                            &styles.evaluation_dependencies(&path),
                                        wrapper_format,
                                    },
                                )?)
                            } else {
                                None
                            };
                        css_identity =
                            matches!(kind, crate::resource::ResourceKind::Stylesheet { .. })
                                .then(|| styles.identity(&path));
                        let parser_path = prepared_css.as_ref().map_or_else(
                            || crate::resource::parser_path(&path, wrapper_format),
                            |prepared| prepared.parser_path.clone(),
                        );
                        let parser_package = prepared_css
                            .as_ref()
                            .map_or(format, |prepared| prepared.package);
                        let text = if let Some(prepared) = prepared_css {
                            relocations.insert(path.clone(), prepared.relocations);
                            prepared.source
                        } else {
                            crate::resource::prepare(&path, &input, &public_base, wrapper_format)?
                        };
                        prepare::load(
                            &path,
                            input,
                            text,
                            &parser_path,
                            parser_package,
                            previous.get(&path),
                            options.cache,
                            &declarations,
                        )?
                    }
                };
                if previous
                    .get(&path)
                    .is_some_and(|source| source.css_identity != css_identity)
                {
                    changed.insert(path.clone());
                }
                match loaded.status {
                    None => counters.memory_hits += 1,
                    Some(CacheStatus::Hit) => counters.disk_hits += 1,
                    Some(CacheStatus::Miss(_)) => counters.source_parses += 1,
                }
                if let Some(warning) = loaded.warning {
                    warnings.push(warning);
                }
                if let Some(mut source) = loaded.source {
                    changed.insert(path.clone());
                    source.css_identity = css_identity.clone();
                    self.resolutions.remove(&path);
                    loaded_sources.insert(path.clone(), source);
                }
                let source = loaded_sources
                    .get(&path)
                    .or_else(|| previous.get(&path))
                    .expect("loaded source");
                let resolution_span = tracing::info_span!("build.resolve").entered();
                let resolved = if let Some(cached) = self.resolutions.get(&path).filter(|_| !external_sources) {
                    cached.clone()
                } else {
                    let mut resolved = HashMap::new();
                    for dependency in &source.record.module.facts.dependencies {
                        if dependency.type_only {
                            continue;
                        }
                        counters.resolutions += 1;
                        let mode = if dependency.kind == DependencyKind::Require {
                            ResolutionMode::Require
                        } else {
                            ResolutionMode::Import
                        };
                        if dependency.pattern.is_some() {
                            let candidates = crate::context::resolve(
                                &self.root,
                                &path,
                                dependency,
                                inventory.candidates(&self.root, &path, dependency),
                                &options.resolver.extensions,
                                &resolver,
                            )
                            .wrap_err_with(|| {
                                format!(
                                    "{}: context at bytes {}..{}",
                                    path.display(),
                                    dependency.span.start,
                                    dependency.span.end
                                )
                            })?;
                            resolved.insert(dependency.id, DependencyTarget::Context(candidates));
                            continue;
                        }
                        let target = resolver
                            .resolve_dependency(&path, &dependency.specifier, mode)
                            .wrap_err_with(|| {
                                format!(
                                    "{}: bytes {}..{}",
                                    path.display(),
                                    dependency.span.start,
                                    dependency.span.end
                                )
                            })?;
                        resolved.insert(
                            dependency.id,
                            match target {
                                ResolvedDependency::File(target) => {
                                    ensure!(inventory.contains(&target),
                                        "{}: resolving {:?}: source outside reachable build inventory (or reserved/excluded): {}",
                                        path.display(), dependency.specifier, target.display());
                                    DependencyTarget::File(target)
                                },
                                ResolvedDependency::External(name) => {
                                    DependencyTarget::External(name)
                                }
                            },
                        );
                    }
                    if self.resolutions.get(&path) != Some(&resolved) {
                        changed.insert(path.clone());
                    }
                    self.resolutions.insert(path.clone(), resolved.clone());
                    resolved
                };
                drop(resolution_span);
                for target in resolved.values() {
                    if let DependencyTarget::File(path) = target {
                        discovered.insert(path.clone());
                    } else if let DependencyTarget::Context(candidates) = target {
                        discovered.extend(candidates.values().cloned());
                    }
                }
                let graph_input = if kind == crate::resource::ResourceKind::Script {
                    OwnedSourceFileInput::from_parsed
                } else {
                    OwnedSourceFileInput::from_prepared_resource
                };
                inputs.push(graph_input(
                    path.clone(),
                    loaded.parsed,
                    resolved.into_iter().collect(),
                ));
                sources.insert(path.clone(), source.text.clone());
                Ok(())
            })();
            let failed = result.is_err();
            if let Err(error) = result {
                errors.insert(path, error);
            }
            (discovered, failed)
        };
        let preparation_span = tracing::info_span!("build.prepare").entered();
        'frontier: loop {
            let result = options.execution.replenish(jobs, &read, |result| {
                let (discovered, _) = integrate(result);
                prepare::enqueue(discovered, &mut scheduled, &mut resources, options)
            });
            if let Err(error) = result {
                self.resolutions.clear();
                self.pending_invalidated.extend(changed);
                return Err(error);
            }
            // Stateful resources have a canonical serial frontier, not a buffer
            // of worker-read bytes. Scripts above replenish without wave barriers.
            jobs = Vec::new();
            while let Some(path) = resources.pop_first() {
                let (discovered, failed) = integrate(read(path));
                if failed {
                    break 'frontier;
                }
                jobs = prepare::enqueue(discovered, &mut scheduled, &mut resources, options);
                if !jobs.is_empty() {
                    break;
                }
            }
            if jobs.is_empty() {
                break;
            }
        }
        drop(preparation_span);
        self.sources.extend(loaded_sources);
        // CSS closure identity can change while its generated module stays equal.
        for path in sources.keys() {
            if matches!(
                crate::resource::kind(path, &options.css.css_modules),
                crate::resource::ResourceKind::Stylesheet { .. }
            ) {
                self.sources
                    .get_mut(path)
                    .expect("loaded source")
                    .css_identity = Some(styles.identity(path));
            }
        }
        if resolution_changed {
            changed.extend(self.sources.keys().cloned());
        }
        self.pending_invalidated.extend(changed);
        if let Some((_, error)) = errors.into_iter().next() {
            return Err(error);
        }
        warnings.sort();
        for path in self.sources.keys() {
            if !sources.contains_key(path) {
                self.pending_invalidated.insert(path.clone());
            }
        }
        let invalidated = self.reverse_users(self.pending_invalidated.clone());
        self.sources.retain(|path, _| sources.contains_key(path));
        self.resolutions
            .retain(|path, _| sources.contains_key(path));
        let fingerprint_span = tracing::info_span!("build.fingerprints").entered();
        let parse = Fingerprint::of(
            format!(
                "{}:{}:{}:{declarations_key:?}:{:?}",
                segment_cache::SCHEMA_VERSION,
                segment_cache::EXTRACTOR_VERSION,
                segment_cache::OXC_REVISION,
                self.sources
                    .iter()
                    .map(|(p, s)| (p, s.record.source_hash, s.record.config))
                    .collect::<Vec<_>>()
            )
            .as_bytes(),
        );
        let resolution_inputs: Vec<_> = self
            .resolutions
            .iter()
            .map(|(path, targets)| {
                let mut targets: Vec<_> =
                    targets.iter().map(|(id, target)| (id.0, target)).collect();
                targets.sort_by_key(|(id, _)| *id);
                (
                    path,
                    &self.sources[path].record.module.facts.dependencies,
                    targets,
                )
            })
            .collect();
        let mut fingerprints = Fingerprints {
            parse,
            resolution: Fingerprint::of(
                format!("{resolution_key:?}:{resolution_inputs:?}").as_bytes(),
            ),
            effects: options.effects.fingerprint(),
            roots: Fingerprint::of(format!("{entries:?}").as_bytes()),
            runtime: Fingerprint::of(
                concat!(
                    "native-node-cjs-initializers-v1\n",
                    include_str!("native_cjs.rs"),
                    include_str!("runtime.cjs"),
                    include_str!("browser_intrinsics.mjs"),
                    include_str!("context_intrinsics.mjs"),
                    include_str!("commonjs_namespace.mjs")
                )
                .as_bytes(),
            ),
            output: Fingerprint::default(),
        };
        // Include emitter implementation, not user-chosen cache-busting strings.
        let emitter = Fingerprint::of(
            concat!(
                include_str!("link.rs"),
                include_str!("assembly.rs"),
                include_str!("entry_outputs.rs"),
                include_str!("symbols.rs"),
                include_str!("native.rs"),
                include_str!("native_chunks.rs"),
                include_str!("native_libraries.rs"),
                include_str!("native_segments.rs"),
                include_str!("native_cjs.rs"),
                include_str!("cjs.rs"),
                include_str!("context.rs"),
                include_str!("resource.rs"),
                include_str!("css.rs"),
                include_str!("css_loading.rs"),
                include_str!("css_selection.rs"),
                include_str!("css_syntax.rs"),
                include_str!("css_names.rs"),
                include_str!("output_url.rs"),
                include_str!("emit.rs"),
                include_str!("rewrite.rs"),
                include_str!("lib.rs"),
                include_str!("chunks.rs"),
                include_str!("async_boundaries.rs"),
                include_str!("manifest.rs"),
                include_str!("effects.rs"),
                include_str!("effect_analysis.rs"),
                include_str!("effect_dependencies.rs")
            )
            .as_bytes(),
        );
        fingerprints.output = Fingerprint::of(
            format!(
                "{fingerprints:?}:{emitter:?}:{}:{:?}:{}:{:?}:{:?}:{}:{}",
                crate::css_syntax::PARSER_REVISION,
                options.entries,
                public_base.as_str(),
                options.css,
                styles.identities(),
                options.output.min_chunk_bytes,
                options.output.synthesize_async_boundaries,
            )
            .as_bytes(),
        );
        drop(fingerprint_span);
        if let Some(last) = &self.last {
            if last.fingerprints == fingerprints {
                Self::recheck(&self.root, &self.sources, &inventory)?;
                resolver.recheck(&self.root, options)?;
                styles.recheck()?;
                counters.record_parses(before_parses);
                counters.elapsed = start.elapsed();
                warnings.extend(last.plan.retention.warnings.iter().cloned());
                self.pending_invalidated.clear();
                return Ok(BuildResult {
                    product: last.clone(),
                    counters,
                    invalidated,
                    warnings,
                });
            }
        }
        // shortcut: rebuild derived graph/retention/output after any key change;
        // optimize only after benchmarks justify a finer later-layer worklist.
        let graph_span = tracing::info_span!("build.graph", modules = inputs.len()).entered();
        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        let graph = SourceGraph::from_owned(inputs);
        graph.validate_links()?;
        drop(graph_span);
        let roots: Vec<_> = entries
            .iter()
            .map(|(name, (path, namespace))| {
                let file = graph.file_id(path).expect("entry was loaded");
                RootSeed {
                    id: RootId::Entry(name.clone()),
                    evaluation_roots: vec![file],
                    binding_roots: if *namespace {
                        vec![GraphNode::Module(file)]
                    } else {
                        vec![]
                    },
                }
            })
            .collect();
        let plan = tracing::info_span!("build.plan").in_scope(||
            chunks::partition_with_options(&graph, &roots, &options.effects, &options.output))?;
        warnings.extend(plan.retention.warnings.iter().cloned());
        let mut artifacts = link::esm_plan_configured(
            &graph,
            &plan,
            &options.effects,
            &sources,
            &relocations,
            options.resolver.target_env == TargetEnv::Browser,
            &require_defaults,
            &BTreeMap::new(),
            options.output.min_chunk_bytes,
            options.execution,
        )?;
        let finalize_span = tracing::info_span!("build.finalize").entered();
        for &file in &plan.retention.modules {
            let path = graph.file_path(file).expect("retained source");
            let kind = crate::resource::kind(path, &options.css.css_modules);
            if kind == crate::resource::ResourceKind::Asset {
                let input = &self.sources[path].input;
                artifacts
                    .resources
                    .insert(crate::resource::asset_path(path, input), input.clone());
            } else if matches!(kind, crate::resource::ResourceKind::Stylesheet { .. }) {
                styles.emit(path, &mut artifacts)?;
            }
        }
        let destinations = entry_outputs::apply(&self.root, options, &mut artifacts)?;
        let artifacts = artifacts.render_to(&destinations)?;
        drop(finalize_span);
        Self::recheck(&self.root, &self.sources, &inventory)?;
        resolver.recheck(&self.root, options)?;
        styles.recheck()?;
        counters.emissions = 1;
        counters.record_parses(before_parses);
        let generation = match &self.last {
            Some(last) => last
                .generation
                .checked_add(1)
                .ok_or_else(|| eyre::eyre!("build generation exhausted"))?,
            None => 1,
        };
        let product = Rc::new(BuildProduct {
            generation,
            graph,
            plan,
            artifacts,
            fingerprints,
        });
        self.last = Some(product.clone());
        self.pending_invalidated.clear();
        counters.elapsed = start.elapsed();
        Ok(BuildResult {
            product,
            counters,
            invalidated,
            warnings,
        })
    }

    fn recheck(
        root: &Path,
        sources: &BTreeMap<PathBuf, Source>,
        before: &Inventory,
    ) -> eyre::Result<()> {
        let _span = tracing::info_span!("build.recheck", sources = sources.len()).entered();
        for (path, source) in sources {
            ensure!(
                fs::read(path)? == source.input,
                "source changed during build: {}",
                path.display()
            );
        }
        ensure!(
            inventory(root)? == *before,
            "resolution inputs changed during build"
        );
        Ok(())
    }

    fn reverse_users(&self, mut changed: BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
        if let Some(last) = &self.last {
            let graph = &last.graph;
            let mut queue: VecDeque<_> = changed.iter().cloned().collect();
            while let Some(path) = queue.pop_front() {
                if let Some(file) = graph.file_id(&path) {
                    let nodes = std::iter::once(GraphNode::Module(file)).chain(
                        graph
                            .file_segments(file)
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .map(|(i, _)| {
                                GraphNode::Segment(source_graph::SegmentKey::new(file, i as u32))
                            }),
                    );
                    for node in nodes {
                        for edge in graph.incoming(node) {
                            let from = match edge.from {
                                GraphNode::Module(f) | GraphNode::Unlinked { file_id: f, .. } => f,
                                GraphNode::Segment(k) => k.file_id,
                            };
                            let path = graph
                                .file_path(from)
                                .expect("live incoming source")
                                .to_owned();
                            if changed.insert(path.clone()) {
                                queue.push_back(path);
                            }
                        }
                    }
                }
            }
        }
        changed
    }
}

fn package_format(root: &Path, source: &Path) -> eyre::Result<ModuleFormat> {
    for directory in source.parent().into_iter().flat_map(Path::ancestors) {
        let package = directory.join("package.json");
        match fs::read(&package) {
            Ok(bytes) => {
                let json: serde_json::Value = serde_json::from_slice(&bytes)
                    .wrap_err_with(|| package.display().to_string())?;
                return match json.get("type").and_then(|v| v.as_str()) {
                    Some("module") => Ok(ModuleFormat::Esm),
                    Some("commonjs") | None => Ok(ModuleFormat::CommonJs),
                    Some(other) => {
                        eyre::bail!("{}: unsupported package type {other}", package.display())
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if directory == root {
            break;
        }
    }
    Ok(ModuleFormat::CommonJs)
}

mod inventory;
mod resolver;
use inventory::inventory;
pub(crate) use inventory::Inventory;

#[cfg(test)]
mod tests;
