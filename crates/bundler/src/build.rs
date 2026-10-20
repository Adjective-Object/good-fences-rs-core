//! Incremental orchestration over the shared parse, resolver, graph and linker.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use ast_segmenter::{owned::DependencyKind, ModuleFormat};
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
    /// Logical entry IDs mapped to source entries and optional output names.
    pub entries: BTreeMap<String, Entry>,
    pub resolver: NodeModulesResolverOptions,
    pub externals: BTreeSet<String>,
    pub effects: EffectPolicy,
    pub declarations: ecco::DeclarationConfig,
    pub cache: CacheMode,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            resolver: NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
            externals: BTreeSet::new(),
            effects: EffectPolicy::default(),
            declarations: ecco::DeclarationConfig::default(),
            cache: CacheMode::ReadWrite,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprints {
    pub parse: [u8; 32],
    pub resolution: [u8; 32],
    pub effects: [u8; 32],
    pub roots: [u8; 32],
    pub runtime: [u8; 32],
    pub output: [u8; 32],
}

#[derive(Debug, Default, Clone)]
pub struct Counters {
    /// Actual source extraction cache misses (emission checks are separate).
    pub source_parses: usize,
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
    fn record_parses(&mut self, before: (usize, usize, usize)) {
        self.source_reparses = ast_segmenter::module_parse_count() - before.0 - self.source_parses;
        self.final_validations = crate::rewrite::final_validation_count() - before.2;
        self.fragment_reparses =
            crate::rewrite::output_parse_count() - before.1 - self.final_validations;
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
    text: String,
    record: CacheRecord,
}

/// One caller owns a session (`&mut self`); independent sessions share sidecars
/// safely. Failed builds never replace the last successful product.
pub struct BuildSession {
    root: PathBuf,
    sources: BTreeMap<PathBuf, Source>,
    resolutions: BTreeMap<PathBuf, HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>>,
    resolution_key: Option<[u8; 32]>,
    last: Option<Rc<BuildProduct>>,
}

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
            last: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn build(&mut self, options: &BuildOptions) -> eyre::Result<BuildResult> {
        let start = Instant::now();
        ensure!(
            !options.entries.is_empty(),
            "build requires at least one entry"
        );
        ensure!(
            !options.resolver.preserve_symlinks,
            "build supports canonical module identities only"
        );
        ensure!(
            options.resolver.target_env == TargetEnv::Node,
            "public build target is Node only"
        );
        let mut counters = Counters::default();
        let before_parses = (
            ast_segmenter::module_parse_count(),
            crate::rewrite::output_parse_count(),
            crate::rewrite::final_validation_count(),
        );
        let mut warnings = Vec::new();
        let declarations = options.declarations.effective()?;
        let declarations_key = hash(&serde_json::to_vec(&declarations)?);
        let inventory = inventory(&self.root)?;
        let mut aliases: Vec<_> = options.resolver.alias.iter().collect();
        aliases.sort();
        let resolution_key = hash(
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
            self.resolution_key = Some(resolution_key);
        }
        let resolver = StructuralResolver::new(
            self.root.clone(),
            options.resolver.clone(),
            options.externals.clone(),
        );
        let mut queue = VecDeque::new();
        let mut entries = BTreeMap::new();
        for (name, entry) in &options.entries {
            ensure!(!name.is_empty(), "empty entry name");
            let path = self
                .root
                .join(&entry.path)
                .canonicalize()
                .wrap_err_with(|| format!("entry {}: {}", name, entry.path.display()))?;
            ensure!(path.starts_with(&self.root), "entry outside build root");
            entries.insert(name.clone(), (path.clone(), entry.namespace));
            queue.push_back(path);
        }
        let mut inputs = Vec::new();
        let mut sources = BTreeMap::new();
        let mut changed = BTreeSet::new();
        while let Some(path) = queue.pop_front() {
            if sources.contains_key(&path) {
                continue;
            }
            ensure!(
                path.starts_with(&self.root),
                "resolved source outside build root: {}",
                path.display()
            );
            ensure!(
                inventory.contains_key(path.strip_prefix(&self.root)?),
                "source is inside a reserved/excluded build directory: {}",
                path.display()
            );
            let format = package_format(&self.root, &path)?;
            let config = ParserConfig::for_path(&path, format)?;
            let text =
                fs::read_to_string(&path).wrap_err_with(|| format!("source {}", path.display()))?;
            if self.sources.get(&path).is_some_and(|s| {
                s.text == text
                    && s.record.config == config
                    && s.record.matches_declarations(&declarations)
            }) {
                counters.memory_hits += 1;
            } else {
                let loaded = segment_cache::load_or_parse_with_declarations(
                    &logger::StdioLogger::new(),
                    &path,
                    format,
                    options.cache,
                    &declarations,
                )?;
                match loaded.status {
                    CacheStatus::Hit => counters.disk_hits += 1,
                    CacheStatus::Miss(_) => counters.source_parses += 1,
                }
                if let CacheWrite::Failed(message) = loaded.write {
                    warnings.push(format!("{}: cache write: {message}", path.display()));
                }
                changed.insert(path.clone());
                self.resolutions.remove(&path);
                self.sources.insert(
                    path.clone(),
                    Source {
                        text: loaded.source,
                        record: loaded.record,
                    },
                );
            }
            let source = &self.sources[&path];
            let resolved = if let Some(cached) = self.resolutions.get(&path) {
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
                            inventory.keys(),
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
                            ResolvedDependency::File(path) => DependencyTarget::File(path),
                            ResolvedDependency::External(name) => DependencyTarget::External(name),
                        },
                    );
                }
                self.resolutions.insert(path.clone(), resolved.clone());
                resolved
            };
            for target in resolved.values() {
                if let DependencyTarget::File(path) = target {
                    queue.push_back(path.clone());
                } else if let DependencyTarget::Context(candidates) = target {
                    queue.extend(candidates.values().cloned());
                }
            }
            inputs.push(OwnedSourceFileInput::from_parsed(
                path.clone(),
                source.record.to_parsed(&source.text)?,
                resolved.into_iter().collect(),
            ));
            sources.insert(path, source.text.clone());
        }
        for path in self.sources.keys() {
            if !sources.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        if resolution_changed {
            changed.extend(self.sources.keys().cloned());
        }
        let invalidated = self.reverse_users(changed);
        self.sources.retain(|path, _| sources.contains_key(path));
        self.resolutions
            .retain(|path, _| sources.contains_key(path));
        let parse = hash(
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
            resolution: hash(format!("{resolution_key:?}:{resolution_inputs:?}").as_bytes()),
            effects: options.effects.fingerprint(),
            roots: hash(format!("{entries:?}").as_bytes()),
            runtime: hash(
                concat!(
                    "native-node-cjs-initializers-v1\n",
                    include_str!("native_cjs.rs"),
                    include_str!("runtime.cjs"),
                    include_str!("commonjs_namespace.mjs")
                )
                .as_bytes(),
            ),
            output: [0; 32],
        };
        // Include emitter implementation, not user-chosen cache-busting strings.
        let emitter = hash(
            concat!(
                include_str!("link.rs"),
                include_str!("assembly.rs"),
                include_str!("entry_outputs.rs"),
                include_str!("symbols.rs"),
                include_str!("native.rs"),
                include_str!("native_chunks.rs"),
                include_str!("native_segments.rs"),
                include_str!("native_cjs.rs"),
                include_str!("cjs.rs"),
                include_str!("context.rs"),
                include_str!("emit.rs"),
                include_str!("rewrite.rs"),
                include_str!("lib.rs"),
                include_str!("chunks.rs"),
                include_str!("manifest.rs"),
                include_str!("effects.rs"),
                include_str!("effect_analysis.rs"),
                include_str!("effect_dependencies.rs")
            )
            .as_bytes(),
        );
        fingerprints.output =
            hash(format!("{fingerprints:?}:{emitter:?}:{:?}", options.entries).as_bytes());
        if let Some(last) = &self.last {
            if last.fingerprints == fingerprints {
                Self::recheck(&self.root, &sources, &inventory)?;
                counters.record_parses(before_parses);
                counters.elapsed = start.elapsed();
                warnings.extend(last.plan.retention.warnings.iter().cloned());
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
        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        let graph = SourceGraph::from_owned(inputs);
        graph.validate_links()?;
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
        let plan = chunks::partition(&graph, &roots, &options.effects)?;
        warnings.extend(plan.retention.warnings.iter().cloned());
        let mut artifacts = link::esm_plan(&graph, &plan, &options.effects, &sources)?;
        let destinations = entry_outputs::apply(&self.root, options, &mut artifacts)?;
        let artifacts = artifacts.render_to(&destinations)?;
        Self::recheck(&self.root, &sources, &inventory)?;
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
        sources: &BTreeMap<PathBuf, String>,
        before: &BTreeMap<PathBuf, Option<[u8; 32]>>,
    ) -> eyre::Result<()> {
        for (path, text) in sources {
            ensure!(
                fs::read(path)? == text.as_bytes(),
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

/// Conservative resolver dependency inventory: all file names and all JSON
/// config bytes, including package trees. Validate direct tsconfig conditions.
fn inventory(root: &Path) -> eyre::Result<BTreeMap<PathBuf, Option<[u8; 32]>>> {
    fn visit(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<PathBuf, Option<[u8; 32]>>,
    ) -> eyre::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git"
                || name == "target"
                || name.to_string_lossy().starts_with(".bundler-")
                || segment_cache::is_cache_artifact(&path)
            {
                continue;
            }
            let ty = entry.file_type()?;
            ensure!(
                !ty.is_symlink(),
                "symlinks in build inventory unsupported: {}",
                path.display()
            );
            if ty.is_dir() {
                visit(root, &path, out)?;
            } else if ty.is_file() {
                if name == "tsconfig.json" {
                    import_resolver::node_resolver::read_custom_conditions(&path)?;
                }
                out.insert(
                    path.strip_prefix(root)?.to_owned(),
                    if path
                        .extension()
                        .is_some_and(|e| e == "json" || e == "jsonc")
                    {
                        Some(hash(&fs::read(path)?))
                    } else {
                        None
                    },
                );
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests;
