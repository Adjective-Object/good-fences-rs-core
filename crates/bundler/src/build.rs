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

mod discovery;
#[path = "entry_outputs.rs"]
mod entry_outputs;
mod policy;
mod prepare;
pub use policy::{PolicyAllowance, PolicyName, PreparationErrors};
mod scratch;
mod worker_entries;

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
    /// Explicit bounded workers; defaults to one preparation/resolution worker.
    pub execution: crate::execution::ExecutionOptions,
    /// Logical entry IDs mapped to source entries and optional output names.
    pub entries: BTreeMap<String, Entry>,
    pub resolver: NodeModulesResolverOptions,
    pub externals: BTreeSet<String>,
    pub effects: EffectPolicy,
    /// JavaScript and target-selected host contracts, followed by exact user overrides.
    pub declarations: ecco::DeclarationConfig,
    pub cache: CacheMode,
    /// Exact source/policy waivers; evidence is admitted anew on every build/hit.
    pub policy_allowlist: Vec<PolicyAllowance>,
    pub output: OutputOptions,
    pub css: CssOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CssOptions {
    pub load: CssLoad,
    pub css_modules: CssModules,
    /// Dotted filename suffixes excluded from module scoping, even when selected.
    pub css_modules_exclude: Vec<String>,
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
    /// Exact runtime identities authorizing lazy graph overlays; empty disables them.
    /// Invalid identities or conflicting constructor roles are configuration errors.
    pub lazy_runtimes: Vec<crate::lazy_modules::LazyRuntime>,
    /// Emit external JavaScript source maps with embedded original source text.
    pub source_maps: bool,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            base_url: "./".into(),
            min_chunk_bytes: 0,
            synthesize_async_boundaries: false,
            lazy_runtimes: Vec::new(),
            source_maps: false,
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
            policy_allowlist: Vec::new(),
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
    /// Actual demand/context directory enumeration, including final rechecks.
    pub inventory_directory_reads: usize,
    /// Actual source extraction cache misses (emission checks are separate).
    pub source_parses: usize,
    /// Speculative extractions not consumed by the precise source frontier.
    pub speculative_parses: usize,
    /// Actual declaration serializations/hashes, including worker-local calls.
    pub declaration_fingerprints: usize,
    pub memory_hits: usize,
    pub disk_hits: usize,
    /// Actual scoped StructuralResolver calls, excluding proven request hits.
    pub resolutions: usize,
    /// Literal/context/resource requests authorized by scoped original proofs.
    pub resolution_cache_hits: usize,
    pub resolution_cache_misses: usize,
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
        self.source_reparses = counts.source - self.source_parses - self.speculative_parses;
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
    /// Nonfatal cache failures, broad dynamic captures and effect-order cycles.
    /// Source/plan warnings repeat on no-op reuse independently of cache state.
    pub warnings: Vec<String>,
}

struct Source {
    css_identity: Option<PathBuf>,
    text: crate::prepared_text::PreparedText,
    record: CacheRecord,
    transpiled: Option<Transpiled>,
}

impl link::SourceTexts for BTreeMap<PathBuf, Source> {
    fn text(&self, path: &Path) -> eyre::Result<Option<crate::prepared_text::Text>> {
        self.get(path)
            .map(|source| source.text.text(path))
            .transpose()
    }
}

#[derive(Clone)]
struct Transpiled {
    config: ParserConfig,
    source_map: Option<cache_flatbuffers::WireSourceMap>,
    source_map_hint: Option<oxc_span::Span>,
}

/// One caller owns a session (`&mut self`); independent sessions share sidecars
/// safely. Failed builds never replace the last successful product.
pub struct BuildSession {
    root: PathBuf,
    sources: BTreeMap<PathBuf, Source>,
    upstreams: BTreeMap<PathBuf, crate::upstream::Upstream>,
    resolutions: BTreeMap<PathBuf, HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>>,
    resolution_memory: resolver::Memory,
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
        let root = root
            .as_ref()
            .canonicalize()
            .wrap_err_with(|| format!("opening build root {}", root.as_ref().display()))?;
        ensure!(
            root.is_dir(),
            "build root is not a directory: {}",
            root.display()
        );
        Ok(Self {
            root,
            sources: BTreeMap::new(),
            upstreams: BTreeMap::new(),
            resolutions: BTreeMap::new(),
            resolution_memory: resolver::Memory::default(),
            declarations: None,
            pending_invalidated: BTreeSet::new(),
            last: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn build(&mut self, options: &BuildOptions) -> eyre::Result<BuildResult> {
        options
            .execution
            .scoped_pool(|| self.build_inner(options))?
            .wrap_err_with(|| format!("building project {}", self.root.display()))
    }

    fn build_inner(&mut self, options: &BuildOptions) -> eyre::Result<BuildResult> {
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
        let public_base = crate::output_url::PublicBase::new(&options.output.base_url)
            .wrap_err("validating output public base URL")?;
        options.css.validate().wrap_err("validating CSS options")?;
        tracing::info!(
            target: "bundler::progress",
            entries = options.entries.len(),
            workers = options.execution.workers().get(),
            "Preparing effect declarations"
        );
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
                crate::effect_analysis::prepare_declarations(&options.declarations, environment)
                    .wrap_err("preparing effect declarations")?;
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
        let inventory = Inventory::new(&self.root);
        let mut styles = crate::css::Styles::new(
            &self.root,
            &inventory,
            &public_base,
            &options.css,
            &options.resolver,
            options.output.source_maps,
        );
        let resolution_fingerprint_span = tracing::info_span!("build.resolver_policy").entered();
        let resolution_key = resolver::BuildResolver::policy(options);
        drop(resolution_fingerprint_span);
        tracing::info!(target: "bundler::progress", "Loading resolver and lockfile");
        let lockfile = resolver::Lockfile::read(&self.root, &mut warnings, options.execution)?;
        let herd = bumpalo_herd::Herd::new();
        let scratch = scratch::Scratch::new(&herd);
        let resolver = resolver::BuildResolver::new(
            &self.root,
            options,
            lockfile.clone(),
            self.resolution_memory.clone(),
            &scratch,
        )
        .admitting(&inventory);
        let mut sass_options = options.clone();
        sass_options.resolver = resolver::BuildResolver::options(options);
        sass_options.resolver.extensions = vec!["scss".into(), "sass".into(), "css".into()];
        sass_options
            .resolver
            .export_conditions
            .extend(["sass".into(), "style".into()]);
        let sass_resolver = resolver::BuildResolver::new(
            &self.root,
            &sass_options,
            lockfile,
            self.resolution_memory.clone(),
            &scratch,
        )
        .admitting(&inventory);
        let mut queue = VecDeque::new();
        let mut entries = BTreeMap::new();
        let mut errors: BTreeMap<PathBuf, Vec<eyre::Report>> = BTreeMap::new();
        for (name, entry) in &options.entries {
            ensure!(!name.is_empty(), "empty entry name");
            let requested = self.root.join(&entry.path);
            let admitted = (|| -> eyre::Result<PathBuf> {
                let path = inventory
                    .admit(&requested)
                    .wrap_err_with(|| format!("entry {}: {}", name, entry.path.display()))?;
                ensure!(
                    inventory.contains(&path),
                    "entry outside reachable build inventory: {}",
                    path.display()
                );
                Ok(path)
            })();
            let path = match admitted {
                Ok(path) => path,
                Err(error) => {
                    tracing::error!(target: "bundler::diagnostic", "{error:#}");
                    errors.entry(requested).or_default().push(error);
                    continue;
                }
            };
            entries.insert(name.clone(), (path.clone(), entry.namespace));
            queue.push_back(path);
        }
        let mut entry_options = options.clone();
        let mut scheduled = BTreeSet::new();
        let mut resources = BTreeSet::new();
        let mut awaiting = BTreeSet::new();
        let initial: BTreeSet<_> = queue.into_iter().collect();
        let mut jobs = prepare::enqueue(
            initial,
            &mut scheduled,
            &mut resources,
            &mut awaiting,
            options,
        );
        let candidates = discovery::candidates(
            &self.root,
            entries.values().map(|(path, _)| path.clone()),
            options,
        )?;
        let speculation = prepare::Speculation::new(&candidates, options.execution.workers().get());
        jobs.extend(
            candidates
                .into_iter()
                .filter(|path| !scheduled.contains(path))
                .map(prepare::Job::Speculate),
        );
        let mut prepared_paths = BTreeSet::new();
        let mut loaded_sources = BTreeMap::new();
        let mut loaded_transpiled = BTreeMap::new();
        let previous = &self.sources;
        let mut relocations = BTreeMap::new();
        let mut require_resources = BTreeMap::new();
        let mut changed = BTreeSet::new();
        let mut reported_policies = BTreeSet::new();
        let mut previous_contexts = BTreeMap::new();
        let mut upstreams = BTreeMap::new();
        let mut deferred_styles = BTreeMap::new();
        let mut prepare_styles = false;
        let mut early_requested = BTreeSet::new();
        let mut early_loaded = BTreeMap::new();
        let mut early_resolved = BTreeMap::new();
        let mut early_scheduled = BTreeSet::new();
        let mut speculative_syntax = BTreeMap::new();
        let mut prepared_sources = 0usize;
        let mut resolved_modules = 0usize;
        #[cfg(test)]
        let observer = prepare::AFTER_PREPARE.with(|observer| observer.borrow().clone());
        #[cfg(test)]
        let dependency_observer =
            prepare::AFTER_DEPENDENCIES.with(|observer| observer.borrow().clone());
        let read_with_inventory = |path: PathBuf,
                                   inventory: &Inventory,
                                   dependencies: &dyn Fn(
            &[ast_segmenter::owned::Dependency],
            policy::Admission,
        )| {
            let result = prepare::read(
                &path,
                inventory,
                previous.get(&path),
                options,
                &declarations,
                &public_base,
                dependencies,
            )
            .wrap_err_with(|| format!("preparing module {}", path.display()));
            #[cfg(test)]
            if let Some(observer) = &observer {
                observer(&path);
            }
            (path, result)
        };
        let read = |path| read_with_inventory(path, &inventory, &|_, _| {});
        let work = |job, report: &dyn Fn(prepare::Early)| {
            let provisional = matches!(&job, prepare::Job::PrepareEarly(_));
            match job {
                prepare::Job::Prepare(path) => match speculation.claim(&path) {
                    prepare::Claim::Ready(ready) => (path, Some(ready)),
                    prepare::Claim::Waiting => (path, None),
                    prepare::Claim::Read => {
                        let dependencies = |dependencies: &[ast_segmenter::owned::Dependency],
                                            admission| {
                            report(prepare::Early {
                                path: path.clone(),
                                admission,
                                dependencies: prepare::Dependencies::Early(
                                    dependencies.to_vec().into(),
                                ),
                            });
                            #[cfg(test)]
                            if let Some(observer) = &dependency_observer {
                                observer(&path);
                            }
                        };
                        let (path, result) =
                            read_with_inventory(path.clone(), &inventory, &dependencies);
                        (path, Some((result, Vec::new(), 0)))
                    }
                },
                prepare::Job::Speculate(path) | prepare::Job::PrepareEarly(path) => {
                    match speculation.claim(&path) {
                        prepare::Claim::Ready(ready) => (path, Some(ready)),
                        prepare::Claim::Waiting => (path, None),
                        prepare::Claim::Read => {
                            let candidate_inventory = inventory.fork();
                            let before = crate::rewrite::parse_counts();
                            let admission = if provisional {
                                candidate_inventory.inherit_admitted(&inventory, &path)
                            } else {
                                candidate_inventory.admit(&path)
                            };
                            let result = admission.and_then(|_| {
                                read_with_inventory(
                                    path.clone(),
                                    &candidate_inventory,
                                    &|dependencies, admission| {
                                        if admission.error.is_some()
                                            || !admission.warnings.is_empty()
                                        {
                                            report(prepare::Early {
                                                path: path.clone(),
                                                dependencies: prepare::Dependencies::Early(
                                                    dependencies.to_vec().into(),
                                                ),
                                                admission,
                                            });
                                        }
                                    },
                                )
                                .1
                            });
                            let parses = crate::rewrite::parse_counts().difference(before).source;
                            speculation.record_parses(parses);
                            (
                                path,
                                Some((result, candidate_inventory.observations(), parses)),
                            )
                        }
                    }
                }
                prepare::Job::ResolveSpeculative { path } => {
                    if let Some(record) = speculation.record(&path, previous.get(&path)) {
                        resolver.speculate(&path, &record);
                    }
                    (path, None)
                }
                prepare::Job::ResolveEarly { path, dependencies } => {
                    let resolutions =
                        prepare::resolve(&path, &dependencies, &inventory, options, &resolver);
                    (
                        path,
                        Some((
                            Ok(prepare::Prepared::EarlyResolved { resolutions }),
                            Vec::new(),
                            0,
                        )),
                    )
                }
                prepare::Job::Resolve {
                    path,
                    loaded,
                    css_identity,
                } => {
                    let record = &loaded
                        .source
                        .as_ref()
                        .or_else(|| previous.get(&path))
                        .expect("prepared source")
                        .record;
                    let resolutions = prepare::resolve(
                        &path,
                        &prepare::Dependencies::Checked(record.clone()),
                        &inventory,
                        options,
                        &resolver,
                    );
                    let result = Ok(prepare::Prepared::Resolved {
                        loaded,
                        css_identity,
                        resolutions,
                    });
                    (path, Some((result, Vec::new(), 0)))
                }
            }
        };
        tracing::info!(target: "bundler::progress", "Preparing modules and resolving dependencies");
        let preparation_span = tracing::info_span!("build.prepare").entered();
        let mut last_progress = Instant::now();
        loop {
            let mut integrate =
                |(path, prepared): (PathBuf, eyre::Result<prepare::Prepared>),
                 deferred_styles: &mut BTreeMap<PathBuf, prepare::Prepared>| {
                    let _span = tracing::info_span!("build.integrate").entered();
                    let mut discovered = BTreeSet::new();
                    let mut provisional = BTreeSet::new();
                    let mut next = None;
                    let result = (|| -> eyre::Result<()> {
                        let prepared = match prepared? {
                            prepare::Prepared::Dependencies {
                                dependencies,
                                admission,
                            } => {
                                admission.report(
                                    &path,
                                    &mut reported_policies,
                                    &mut warnings,
                                    &mut errors,
                                );
                                ensure!(
                                    early_requested.insert(path.clone()),
                                    "duplicate early dependency event"
                                );
                                next = Some(prepare::Job::ResolveEarly {
                                    path: path.clone(),
                                    dependencies,
                                });
                                return Ok(());
                            }
                            prepare::Prepared::EarlyResolved { mut resolutions } => {
                                resolved_modules += 1;
                                resolutions.report_errors(&path, &mut errors);
                                for target in resolutions.targets.values() {
                                    match target {
                                        DependencyTarget::File(path) => {
                                            provisional.insert(path.clone());
                                        }
                                        DependencyTarget::Context(candidates) => {
                                            provisional.extend(candidates.values().cloned())
                                        }
                                        DependencyTarget::External(_)
                                        | DependencyTarget::Missing(_) => {}
                                    }
                                }
                                if errors.get(&path).is_some_and(|reports| {
                                    reports.iter().any(|report| {
                                        report
                                            .downcast_ref::<ast_segmenter::policy::PolicyErrors>()
                                            .is_some()
                                    })
                                }) {
                                    // A demanded policy failure does not invalidate
                                    // its checked dependency syntax.
                                    discovered.extend(provisional.iter().cloned());
                                }
                                early_resolved.insert(path.clone(), resolutions.targets);
                                let Some(loaded) = early_loaded.remove(&path) else {
                                    return Ok(());
                                };
                                prepare::Prepared::Script(loaded)
                            }
                            prepare::Prepared::Script(mut loaded) => {
                                std::mem::take(&mut loaded.admission).report(
                                    &path,
                                    &mut reported_policies,
                                    &mut warnings,
                                    &mut errors,
                                );
                                prepared_sources += 1;
                                if early_requested.contains(&path)
                                    && !early_resolved.contains_key(&path)
                                {
                                    early_loaded.insert(path.clone(), loaded);
                                    return Ok(());
                                }
                                prepare::Prepared::Script(loaded)
                            }
                            prepared => prepared,
                        };
                        let kind = crate::resource::kind(&path, &options.css);
                        if matches!(
                            kind,
                            crate::resource::ResourceKind::Json
                                | crate::resource::ResourceKind::Asset
                        ) || (kind
                            == crate::resource::ResourceKind::Stylesheet { module: false }
                            && options.css.load == CssLoad::CallerLoaded)
                        {
                            require_resources
                                .insert(path.clone(), crate::resource::RequireValue::Default);
                        } else if kind
                            == (crate::resource::ResourceKind::Stylesheet { module: true })
                            && options.css.load == CssLoad::CallerLoaded
                        {
                            require_resources
                                .insert(path.clone(), crate::resource::RequireValue::Namespace);
                        }
                        let wrapper_format = match options.resolver.target_env {
                            TargetEnv::Browser => ModuleFormat::Esm,
                            TargetEnv::Node => ModuleFormat::CommonJs,
                        };
                        let mut css_identity = None;
                        let mut resolutions = None;
                        let loaded = match prepared {
                            prepare::Prepared::Dependencies { .. }
                            | prepare::Prepared::EarlyResolved { .. } => {
                                unreachable!("handled early events")
                            }
                            prepare::Prepared::Resolved {
                                loaded,
                                css_identity: identity,
                                resolutions: mut resolved,
                            } => {
                                resolved.report_errors(&path, &mut errors);
                                css_identity = identity;
                                resolutions = Some(resolved.targets);
                                loaded
                            }
                            prepare::Prepared::Script(loaded) => {
                                if early_requested.contains(&path) {
                                    resolutions = Some(
                                        early_resolved
                                            .remove(&path)
                                            .expect("resolved early dependencies"),
                                    );
                                }
                                loaded
                            }
                            prepare::Prepared::Resource { input, format } => {
                                let prepared_css = if matches!(
                                    kind,
                                    crate::resource::ResourceKind::Stylesheet { .. }
                                ) {
                                    if !prepare_styles {
                                        options.execution.resolve(|| {
                                            styles.discover(
                                                &path,
                                                &input,
                                                &resolver,
                                                &sass_resolver,
                                            )
                                        })?;
                                        discovered.extend(styles.evaluation_paths(&path).cloned());
                                        deferred_styles.insert(
                                            path.clone(),
                                            prepare::Prepared::Resource { input, format },
                                        );
                                        return Ok(());
                                    }
                                    Some(
                                        options.css.load.strategy().generate_module(
                                            &crate::css_loading::CssModuleInput {
                                                stylesheet: &path,
                                                public_url: &styles.public_url(&path)?,
                                                name_mappings: styles
                                                    .name_mappings(&path)
                                                    .unwrap_or(&BTreeMap::new()),
                                                is_module: styles.name_mappings(&path).is_some(),
                                                evaluation_dependencies: &styles
                                                    .evaluation_dependencies(&path),
                                                wrapper_format,
                                            },
                                        )?,
                                    )
                                } else {
                                    None
                                };
                                css_identity = matches!(
                                    kind,
                                    crate::resource::ResourceKind::Stylesheet { .. }
                                )
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
                                    crate::resource::prepare(
                                        &path,
                                        &input,
                                        &public_base,
                                        wrapper_format,
                                    )?
                                };
                                let loaded = prepare::load(
                                    &path,
                                    input,
                                    text,
                                    &parser_path,
                                    parser_package,
                                    previous.get(&path),
                                    options.cache,
                                    (&self.root, &options.policy_allowlist),
                                    &declarations,
                                    &|_, _| {},
                                )?;
                                prepared_sources += 1;
                                loaded
                            }
                        };
                        // Resources and borrowed disk/session owners have no
                        // early syntax event. Admission still precedes resolution.
                        let mut loaded = loaded;
                        std::mem::take(&mut loaded.admission).report(
                            &path,
                            &mut reported_policies,
                            &mut warnings,
                            &mut errors,
                        );
                        let Some(resolved) = resolutions else {
                            next = Some(prepare::Job::Resolve {
                                path: path.clone(),
                                loaded,
                                css_identity,
                            });
                            return Ok(());
                        };
                        if !early_requested.contains(&path) {
                            resolved_modules += 1;
                        }
                        // A later input/map/worker failure must not suppress
                        // other successfully resolved reachable dependencies.
                        for target in resolved.values() {
                            match target {
                                DependencyTarget::File(path) => {
                                    discovered.insert(path.clone());
                                }
                                DependencyTarget::Context(candidates) => {
                                    discovered.extend(candidates.values().cloned());
                                }
                                DependencyTarget::External(_) | DependencyTarget::Missing(_) => {}
                            }
                        }
                        let record = &loaded
                            .source
                            .as_ref()
                            .or_else(|| previous.get(&path))
                            .expect("prepared source")
                            .record;
                        if early_requested.contains(&path)
                            && record
                                .module()
                                .facts()
                                .dependencies()
                                .iter()
                                .any(|dependency| dependency.pattern().is_some())
                        {
                            inventory.seal_context_owner(&path, record);
                        }
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
                            loaded_sources.insert(path.clone(), source);
                        }
                        if let Some(transpiled) = loaded.transpiled {
                            loaded_transpiled.insert(path.clone(), transpiled);
                        }
                        let source = loaded_sources
                            .get(&path)
                            .or_else(|| previous.get(&path))
                            .expect("loaded source");
                        let has_context = source
                            .record
                            .module()
                            .facts()
                            .dependencies()
                            .iter()
                            .any(|dependency| dependency.pattern().is_some());
                        if has_context {
                            previous_contexts
                                .entry(path.clone())
                                .or_insert_with(|| self.resolutions.get(&path).cloned());
                        }
                        if !has_context && self.resolutions.get(&path) != Some(&resolved) {
                            changed.insert(path.clone());
                        }
                        // Late admissions must still expand checked contexts
                        // when subsequent worker/map integration fails.
                        self.resolutions.insert(path.clone(), resolved);
                        let resolved = self.resolutions.get(&path).expect("resolved source");
                        let worker_urls = worker_entries::discover(
                            &path,
                            source,
                            resolved,
                            &inventory,
                            options,
                            &mut entries,
                            &mut entry_options.entries,
                        )?;
                        if !worker_urls.is_empty() {
                            relocations
                                .entry(path.clone())
                                .or_insert_with(Vec::new)
                                .extend(worker_urls);
                        }
                        for dependency in source.record.module().facts().dependencies() {
                            if dependency.broad_context() {
                                let warning = format!(
                                    "{} at bytes {}..{}: unannotated dynamic import captures all files in the importing directory and below; broad recursive capture may increase bundle size",
                                    path.display(), dependency.span().start, dependency.span().end,
                                );
                                tracing::warn!(target: "bundler::progress", "{warning}");
                                warnings.push(warning);
                            }
                        }
                        if options.output.source_maps
                            && kind == crate::resource::ResourceKind::Script
                        {
                            let transpiled =
                                loaded_transpiled.get(&path).or(source.transpiled.as_ref());
                            let original = source.text.read_input(&path)?;
                            let (text, hint) = if let Some(transpiled) = transpiled {
                                (std::str::from_utf8(&original)?, transpiled.source_map_hint)
                            } else {
                                (
                                    std::str::from_utf8(&original)?,
                                    source.record.module().facts().emission().source_map(),
                                )
                            };
                            let (upstream, warning) = crate::upstream::load(
                                &inventory,
                                &path,
                                text,
                                hint,
                                options.cache,
                                self.upstreams.get(&path),
                            )?;
                            if let Some(warning) = warning {
                                warnings.push(warning);
                            }
                            if let Some(upstream) = upstream {
                                if self.upstreams.get(&path) != Some(&upstream) {
                                    changed.insert(path.clone());
                                }
                                upstreams.insert(path.clone(), upstream);
                            } else if self.upstreams.contains_key(&path) {
                                changed.insert(path.clone());
                            }
                        }
                        prepared_paths.insert(path.clone());
                        Ok(())
                    })();
                    let failed = result.is_err();
                    if let Err(error) = result {
                        let error =
                            error.wrap_err(format!("integrating module {}", path.display()));
                        tracing::error!(target: "bundler::diagnostic", "{error:#}");
                        errors
                            .entry(path.clone())
                            .or_insert_with(Vec::new)
                            .push(error);
                    }
                    if last_progress.elapsed() >= Duration::from_secs(1) {
                        tracing::info!(
                            target: "bundler::progress",
                            modules = prepared_paths.len(),
                            prepared_sources,
                            resolved_modules,
                            source_parses = counters.source_parses,
                            memory_hits = counters.memory_hits,
                            disk_hits = counters.disk_hits,
                            "Preparing modules and resolving dependencies"
                        );
                        last_progress = Instant::now();
                    }
                    (discovered, failed, next, provisional)
                };
            loop {
                let result = options.execution.prepare_events(
                    jobs,
                    work,
                    |event| {
                        let (path, mut completed, mut early) = match event {
                            crate::execution::PreparationEvent::Early(prepare::Early {
                                path,
                                dependencies,
                                admission,
                            }) => (
                                path,
                                Some((
                                    Ok(prepare::Prepared::Dependencies {
                                        dependencies,
                                        admission,
                                    }),
                                    Vec::new(),
                                    0,
                                )),
                                true,
                            ),
                            crate::execution::PreparationEvent::Complete((path, completed)) => {
                                (path, completed, false)
                            }
                        };
                        let mut jobs = Vec::new();
                        if early && !scheduled.contains(&path) {
                            // Keep in-flight syntax private. Completion removes
                            // it, so storage is bounded by the worker frontier.
                            if let Some((Ok(prepared), _, _)) = completed {
                                speculative_syntax.insert(path, prepared);
                            }
                            return Ok((jobs, !awaiting.is_empty()));
                        }
                        if !early && completed.is_some() {
                            speculative_syntax.remove(&path);
                        } else if completed.is_none() && scheduled.contains(&path) {
                            // An in-flight speculative owner was demanded. Its
                            // available evidence must not wait for extraction.
                            if let Some(prepared) = speculative_syntax.remove(&path) {
                                completed = Some((Ok(prepared), Vec::new(), 0));
                                early = true;
                            }
                        }
                        if let Some((prepared, observations, parses)) = completed {
                            #[cfg(test)]
                            let resolved_early =
                                matches!(&prepared, Ok(prepare::Prepared::EarlyResolved { .. }));
                            #[cfg(test)]
                            let completed_path = path.clone();
                            if !scheduled.contains(&path) {
                                if matches!(&prepared, Ok(prepare::Prepared::Script(_))) {
                                    jobs.push(prepare::Job::ResolveSpeculative {
                                        path: path.clone(),
                                    });
                                }
                                speculation.store(path, (prepared, observations, parses));
                            } else {
                                if !early {
                                    awaiting.remove(&path);
                                }
                                speculation.promote(parses);
                                let prepared =
                                    inventory.merge_observations(observations).and(prepared);
                                let (discovered, _, next, provisional) =
                                    integrate((path, prepared), &mut deferred_styles);
                                jobs = prepare::enqueue(
                                    discovered,
                                    &mut scheduled,
                                    &mut resources,
                                    &mut awaiting,
                                    options,
                                );
                                jobs.extend(next);
                                jobs.extend(
                                    provisional
                                        .into_iter()
                                        .filter(|path| {
                                            !scheduled.contains(path)
                                                && crate::resource::kind(path, &options.css)
                                                    == crate::resource::ResourceKind::Script
                                                && early_scheduled.insert(path.clone())
                                        })
                                        .map(prepare::Job::PrepareEarly),
                                );
                                #[cfg(test)]
                                if resolved_early {
                                    prepare::EARLY_CHILDREN.with(|observer| {
                                        if let Some(observer) = &*observer.borrow() {
                                            observer(&completed_path, jobs.len());
                                        }
                                    });
                                }
                            }
                        }
                        Ok((jobs, !awaiting.is_empty()))
                    },
                    prepare::Job::priority,
                    prepare::Job::preparation,
                );
                if let Err(error) = result {
                    self.resolutions.clear();
                    self.pending_invalidated.extend(changed);
                    return Err(error);
                }
                // Resource discovery stays canonical/serial; CSS wrappers wait
                // for settled routes. Scripts replenish without wave barriers.
                jobs = Vec::new();
                while let Some(path) = resources.pop_first() {
                    let _resource_span = tracing::info_span!("prepare.resource").entered();
                    let prepared = match deferred_styles.remove(&path) {
                        Some(prepared) => (path, Ok(prepared)),
                        None => read(path),
                    };
                    let (discovered, _, next, _) = integrate(prepared, &mut deferred_styles);
                    jobs = prepare::enqueue(
                        discovered,
                        &mut scheduled,
                        &mut resources,
                        &mut awaiting,
                        options,
                    );
                    jobs.extend(next);
                    if !jobs.is_empty() {
                        break;
                    }
                }
                if jobs.is_empty() {
                    break;
                }
            }
            let expanded = match options
                .execution
                .resolve(|| inventory.expand_contexts(&resolver))
            {
                Ok(expanded) => expanded,
                Err(error) => {
                    tracing::error!(target: "bundler::diagnostic", "{error:#}");
                    errors
                        .entry(self.root.clone())
                        .or_insert_with(Vec::new)
                        .push(error);
                    break;
                }
            };
            if expanded.is_empty() {
                if !deferred_styles.is_empty() {
                    // All checked routes (including maps and composition jobs)
                    // must settle before CSS names, URLs or relocations capture them.
                    if let Err(error) = styles.finish_discovery() {
                        tracing::error!(target: "bundler::diagnostic", "{error:#}");
                        errors
                            .entry(self.root.clone())
                            .or_insert_with(Vec::new)
                            .push(error);
                        break;
                    }
                    prepare_styles = true;
                    resources.extend(deferred_styles.keys().cloned());
                    continue;
                }
                break;
            }
            let mut discovered = BTreeSet::new();
            for (path, dependency, candidates) in expanded {
                discovered.extend(candidates.values().cloned());
                let target = DependencyTarget::Context(candidates);
                self.resolutions
                    .get_mut(&path)
                    .expect("context source")
                    .insert(dependency, target);
            }
            jobs = prepare::enqueue(
                discovered,
                &mut scheduled,
                &mut resources,
                &mut awaiting,
                options,
            );
        }
        drop(preparation_span);
        counters.speculative_parses = speculation.unused_parses();
        for (path, before) in previous_contexts {
            if self.resolutions.get(&path) != before.as_ref() {
                changed.insert(path);
            }
        }
        warnings.extend(styles.warnings().cloned());
        self.sources.extend(loaded_sources);
        for (path, transpiled) in loaded_transpiled {
            self.sources
                .get_mut(&path)
                .expect("loaded prepared source")
                .transpiled = Some(transpiled);
        }
        // CSS closure identity can change while its generated module stays equal.
        for path in &prepared_paths {
            if matches!(
                crate::resource::kind(path, &options.css),
                crate::resource::ResourceKind::Stylesheet { .. }
            ) {
                self.sources
                    .get_mut(path)
                    .expect("loaded source")
                    .css_identity = Some(styles.identity(path));
            }
        }
        self.pending_invalidated.extend(changed);
        if !errors.is_empty() {
            return Err(PreparationErrors::new(errors).into());
        }
        tracing::info!(
            target: "bundler::progress",
            modules = prepared_paths.len(),
            prepared_sources,
            resolved_modules,
            source_parses = counters.source_parses,
            speculative_parses = counters.speculative_parses,
            memory_hits = counters.memory_hits,
            disk_hits = counters.disk_hits,
            "Modules prepared"
        );
        self.upstreams = upstreams;
        warnings.sort();
        for path in self.sources.keys() {
            if !prepared_paths.contains(path) {
                self.pending_invalidated.insert(path.clone());
            }
        }
        let invalidated = self.reverse_users(self.pending_invalidated.clone());
        self.sources.retain(|path, _| prepared_paths.contains(path));
        self.resolutions
            .retain(|path, _| prepared_paths.contains(path));
        resolver.retain_precise();
        sass_resolver.retain_precise();
        tracing::info!(target: "bundler::progress", "Checking build fingerprints");
        let fingerprint_span = tracing::info_span!("build.fingerprints").entered();
        let parse = Fingerprint::of(
            format!(
                "{}:{}:{}:{declarations_key:?}:{:?}",
                segment_cache::SCHEMA_VERSION,
                segment_cache::EXTRACTOR_VERSION,
                segment_cache::OXC_REVISION,
                self.sources
                    .iter()
                    .map(|(p, s)| (p, s.record.source_hash(), s.record.config()))
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
                    self.sources[path].record.module().facts().dependencies(),
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
                include_str!("build/worker_entries.rs"),
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
                include_str!("sourcemap.rs"),
                include_str!("upstream.rs"),
                include_str!("transpile.rs"),
                include_str!("rewrite.rs"),
                include_str!("lib.rs"),
                include_str!("chunks.rs"),
                include_str!("async_boundaries.rs"),
                include_str!("lazy_modules.rs"),
                include_str!("manifest.rs"),
                include_str!("effects.rs"),
                include_str!("effect_analysis.rs"),
                include_str!("effect_dependencies.rs")
            )
            .as_bytes(),
        );
        fingerprints.output = Fingerprint::of(
            format!(
                "{fingerprints:?}:{emitter:?}:{}:{:?}:{}:{:?}:{:?}:{}:{}:{:?}:{}:{:?}:{:?}",
                crate::css_syntax::PARSER_REVISION,
                options.entries,
                public_base.as_str(),
                options.css,
                styles.identities(),
                options.output.min_chunk_bytes,
                options.output.synthesize_async_boundaries,
                crate::lazy_modules::configuration_fingerprint(&options.output.lazy_runtimes),
                options.output.source_maps,
                self.upstreams
                    .iter()
                    .map(|(path, upstream)| (
                        path,
                        cache_flatbuffers::hash128(upstream.map().bytes())
                    ))
                    .collect::<Vec<_>>(),
                options.output.source_maps.then(|| self
                    .sources
                    .iter()
                    .filter_map(|(path, source)| source
                        .transpiled
                        .as_ref()
                        .and_then(|transpiled| transpiled.source_map.as_ref())
                        .map(|map| (path, cache_flatbuffers::hash128(map.bytes()))))
                    .collect::<Vec<_>>()),
            )
            .as_bytes(),
        );
        drop(fingerprint_span);
        if let Some(last) = &self.last {
            if last.fingerprints == fingerprints {
                tracing::info!(target: "bundler::progress", "Rechecking inputs for unchanged build");
                Self::recheck(&self.sources, &inventory)
                    .wrap_err("rechecking sources before build reuse")?;
                counters.inventory_directory_reads = inventory.directory_reads();
                resolver
                    .recheck()
                    .wrap_err("rechecking script resolution inputs before build reuse")?;
                sass_resolver
                    .recheck()
                    .wrap_err("rechecking Sass resolution inputs before build reuse")?;
                styles
                    .recheck()
                    .wrap_err("rechecking stylesheet inputs before build reuse")?;
                for upstream in self.upstreams.values() {
                    upstream
                        .recheck()
                        .wrap_err("rechecking upstream source-map inputs before build reuse")?;
                }
                resolver.finish(&mut counters, &mut warnings);
                sass_resolver.finish(&mut counters, &mut warnings);
                resolver
                    .recheck()
                    .wrap_err("rechecking script inputs after resolution-cache publication")?;
                sass_resolver
                    .recheck()
                    .wrap_err("rechecking Sass inputs after resolution-cache publication")?;
                self.resolution_memory
                    .retain_used(&resolver, &sass_resolver);
                warnings.sort();
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
        tracing::info!(
            target: "bundler::progress",
            modules = prepared_paths.len(),
            "Constructing and validating source graph"
        );
        let graph_span =
            tracing::info_span!("build.graph", modules = prepared_paths.len()).entered();
        let inputs = self
            .sources
            .iter()
            .map(|(path, source)| {
                let text = source.text.text(path)?;
                let parsed = source
                    .record
                    .to_parsed_reporting_policies(&text)
                    .wrap_err_with(|| {
                        format!("borrowing prepared graph facts for {}", path.display())
                    })?;
                let graph_input = if crate::resource::kind(path, &options.css)
                    == crate::resource::ResourceKind::Script
                    && source.transpiled.is_none()
                {
                    OwnedSourceFileInput::from_parsed
                } else {
                    OwnedSourceFileInput::from_prepared_resource
                };
                Ok(graph_input(
                    path.clone(),
                    parsed,
                    self.resolutions[path]
                        .iter()
                        .map(|(id, target)| (*id, target.clone()))
                        .collect(),
                ))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        let graph = SourceGraph::from_owned(inputs);
        graph
            .validate_links()
            .wrap_err("validating prepared source graph")?;
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
        tracing::info!(target: "bundler::progress", "Planning retained modules and chunks");
        let plan = tracing::info_span!("build.plan")
            .in_scope(|| {
                chunks::partition_with_options(&graph, &roots, &options.effects, &options.output)
            })
            .wrap_err("planning retained modules and chunks")?;
        warnings.extend(plan.retention.warnings.iter().cloned());
        let mappings = self
            .sources
            .iter()
            .filter(|(path, _)| {
                options.output.source_maps
                    && crate::resource::kind(path, &options.css)
                        == crate::resource::ResourceKind::Script
            })
            .map(|(path, source)| {
                let compiler_map = source
                    .transpiled
                    .as_ref()
                    .and_then(|transpiled| transpiled.source_map.clone());
                crate::sourcemap::Source::with_text(
                    path,
                    source.text.text(path)?,
                    compiler_map
                        .into_iter()
                        .chain(
                            self.upstreams
                                .get(path)
                                .map(|upstream| upstream.map().clone()),
                        )
                        .collect(),
                )
                .map(|source| (path.clone(), source))
            })
            .collect::<eyre::Result<BTreeMap<_, _>>>()?;
        tracing::info!(target: "bundler::progress", "Linking and emitting native chunks");
        let mut artifacts = link::esm_plan_configured(
            &graph,
            &plan,
            &options.effects,
            &self.sources,
            &relocations,
            options.resolver.target_env == TargetEnv::Browser,
            &require_resources,
            &BTreeMap::new(),
            options.output.min_chunk_bytes,
            &mappings,
            options.execution,
        )
        .wrap_err("linking native output chunks")?;
        tracing::info!(target: "bundler::progress", "Finalizing resources and rendering output");
        let finalize_span = tracing::info_span!("build.finalize").entered();
        for &file in &plan.retention.modules {
            let path = graph.file_path(file).expect("retained source");
            let kind = crate::resource::kind(path, &options.css);
            if kind == crate::resource::ResourceKind::Asset {
                let input = self.sources[path].text.read_input(path)?;
                artifacts
                    .resources
                    .insert(crate::resource::asset_path(path, &input), input);
            } else if matches!(kind, crate::resource::ResourceKind::Stylesheet { .. }) {
                styles
                    .emit(path, &mut artifacts)
                    .wrap_err_with(|| format!("emitting stylesheet {}", path.display()))?;
            }
        }
        for code in artifacts.files.values_mut() {
            code.resolve_entry_urls(&artifacts.entries)?;
        }
        let destinations = entry_outputs::apply(&self.root, &entry_options, &mut artifacts)
            .wrap_err("assigning entry output paths")?;
        let artifacts = artifacts
            .render_to_with_maps(&destinations, options.output.source_maps)
            .wrap_err("rendering output artifacts and source maps")?;
        drop(finalize_span);
        tracing::info!(target: "bundler::progress", "Rechecking build inputs");
        Self::recheck(&self.sources, &inventory)
            .wrap_err("rechecking sources before build success")?;
        counters.inventory_directory_reads = inventory.directory_reads();
        resolver
            .recheck()
            .wrap_err("rechecking script resolution inputs before build success")?;
        styles
            .recheck()
            .wrap_err("rechecking stylesheet inputs before build success")?;
        sass_resolver
            .recheck()
            .wrap_err("rechecking Sass resolution inputs before build success")?;
        for upstream in self.upstreams.values() {
            upstream
                .recheck()
                .wrap_err("rechecking upstream source-map inputs before build success")?;
        }
        resolver.finish(&mut counters, &mut warnings);
        sass_resolver.finish(&mut counters, &mut warnings);
        resolver
            .recheck()
            .wrap_err("rechecking script inputs after resolution-cache publication")?;
        sass_resolver
            .recheck()
            .wrap_err("rechecking Sass inputs after resolution-cache publication")?;
        self.resolution_memory
            .retain_used(&resolver, &sass_resolver);
        warnings.sort();
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

    fn recheck(sources: &BTreeMap<PathBuf, Source>, before: &Inventory) -> eyre::Result<()> {
        let _span = tracing::info_span!("build.recheck", sources = sources.len()).entered();
        for (path, source) in sources {
            source.text.read_input(path)?;
        }
        before
            .recheck()
            .wrap_err("rechecking reachable build inventory")?;
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

mod inventory;
#[cfg(test)]
mod reference_inventory;
mod resolver;
pub(crate) use inventory::Inventory;
#[cfg(test)]
use reference_inventory::inventory;

#[cfg(test)]
mod tests;
