//! Bounded source preparation and import-resolution jobs.
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_PREPARE: std::cell::RefCell<Option<std::sync::Arc<dyn Fn(&Path) + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) enum Job {
    Prepare(PathBuf),
    Speculate(PathBuf),
    ResolveSpeculative {
        path: PathBuf,
        record: CacheRecord,
    },
    Resolve {
        path: PathBuf,
        loaded: Loaded,
        css_identity: Option<PathBuf>,
    },
}

pub(super) type Completion = (
    eyre::Result<Prepared>,
    Vec<ftree_cache::inputs::PathInput>,
    usize,
);

pub(super) enum Claim {
    Read,
    Waiting,
    Ready(Completion),
}

pub(super) struct Speculation {
    claims: BTreeMap<PathBuf, std::sync::atomic::AtomicBool>,
    ready: parking_lot::Mutex<BTreeMap<PathBuf, Completion>>,
    limit: usize,
    parses: std::sync::atomic::AtomicUsize,
    promoted: std::sync::atomic::AtomicUsize,
}

impl Speculation {
    pub(super) fn new(paths: &BTreeSet<PathBuf>, workers: usize) -> Self {
        Self {
            claims: paths
                .iter()
                .map(|path| (path.clone(), false.into()))
                .collect(),
            ready: Default::default(),
            limit: workers.saturating_mul(2),
            parses: 0.into(),
            promoted: 0.into(),
        }
    }

    pub(super) fn record_parses(&self, count: usize) {
        self.parses
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn promote(&self, count: usize) {
        self.promoted
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn unused_parses(&self) -> usize {
        self.parses.load(std::sync::atomic::Ordering::Relaxed)
            - self.promoted.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn claim(&self, path: &Path) -> Claim {
        if let Some(ready) = self.ready.lock().remove(path) {
            return Claim::Ready(ready);
        }
        if self
            .claims
            .get(path)
            .is_some_and(|claimed| claimed.swap(true, std::sync::atomic::Ordering::AcqRel))
        {
            Claim::Waiting
        } else {
            Claim::Read
        }
    }

    pub(super) fn store(&self, path: PathBuf, result: Completion) {
        let mut ready = self.ready.lock();
        ready.insert(path, result);
        while ready.len() > self.limit {
            let (path, _) = ready.pop_first().expect("nonempty speculative cache");
            self.claims[&path].store(false, std::sync::atomic::Ordering::Release);
        }
    }
}

pub(super) enum Prepared {
    Script(Loaded),
    Resource {
        input: Vec<u8>,
        format: ModuleFormat,
    },
    Resolved {
        loaded: Loaded,
        css_identity: Option<PathBuf>,
        resolutions: HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>,
    },
}

pub(super) fn resolve(
    path: &Path,
    record: &CacheRecord,
    inventory: &Inventory,
    options: &BuildOptions,
    resolver: &impl DependencyResolver,
) -> eyre::Result<HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>> {
    let _span = tracing::info_span!("build.resolve").entered();
    let mut resolved = HashMap::new();
    for dependency in record.module().facts().dependencies() {
        if dependency.type_only() {
            continue;
        }
        let _dependency_span = tracing::info_span!(
            "resolve.dependency",
            context = dependency.pattern().is_some(),
            require = dependency.kind() == DependencyKind::Require
        )
        .entered();
        let target = if dependency.pattern().is_some() {
            DependencyTarget::Context(
                crate::context::resolve(
                    inventory.root(),
                    path,
                    dependency,
                    inventory.candidates(path, record, dependency, &options.resolver.extensions)?,
                    &options.resolver.extensions,
                    resolver,
                )
                .wrap_err_with(|| {
                    format!(
                        "{}: context at bytes {}..{}",
                        path.display(),
                        dependency.span().start,
                        dependency.span().end
                    )
                })?,
            )
        } else {
            let mode = if dependency.kind() == DependencyKind::Require {
                ResolutionMode::Require
            } else {
                ResolutionMode::Import
            };
            match resolver
                .resolve_dependency(path, &dependency.specifier(), mode)
                .wrap_err_with(|| {
                    format!(
                        "{}: bytes {}..{}",
                        path.display(),
                        dependency.span().start,
                        dependency.span().end
                    )
                })? {
                ResolvedDependency::File(target) => DependencyTarget::File(target),
                ResolvedDependency::External(name) => DependencyTarget::External(name),
            }
        };
        if dependency.json_attribute().is_some() {
            ensure!(
                matches!(&target, DependencyTarget::File(target)
                    if crate::resource::kind(target, &options.css) == crate::resource::ResourceKind::Json),
                "{}: JSON import attribute at bytes {}..{} requires a bundled JSON resource: {}",
                path.display(),
                dependency.span().start,
                dependency.span().end,
                dependency.specifier()
            );
        }
        resolved.insert(dependency.id(), target);
    }
    Ok(resolved)
}

pub(super) struct Loaded {
    pub source: Option<Source>,
    pub status: Option<CacheStatus>,
    pub warning: Option<String>,
    pub transpiled: Option<Transpiled>,
}

pub(super) fn enqueue(
    paths: impl IntoIterator<Item = PathBuf>,
    scheduled: &mut BTreeSet<PathBuf>,
    resources: &mut BTreeSet<PathBuf>,
    awaiting: &mut BTreeSet<PathBuf>,
    options: &BuildOptions,
) -> Vec<Job> {
    paths
        .into_iter()
        .filter(|path| scheduled.insert(path.clone()))
        .filter(|path| {
            if crate::resource::kind(path, &options.css) == crate::resource::ResourceKind::Script {
                true
            } else {
                resources.insert(path.clone());
                false
            }
        })
        .map(|path| {
            awaiting.insert(path.clone());
            Job::Prepare(path)
        })
        .collect()
}

pub(super) fn read(
    path: &Path,
    inventory: &Inventory,
    previous: Option<&Source>,
    options: &BuildOptions,
    declarations: &PreparedDeclarations,
    base: &crate::output_url::PublicBase,
) -> eyre::Result<Prepared> {
    let _span = tracing::info_span!("prepare.read").entered();
    ensure!(
        inventory.contains(path),
        "source outside reachable build inventory (or reserved/excluded): {}",
        path.display()
    );
    let format = tracing::info_span!("prepare.package_format")
        .in_scope(|| inventory.package_format(path))
        .wrap_err_with(|| format!("determining package format for {}", path.display()))?;
    let input = tracing::info_span!("prepare.read_bytes")
        .in_scope(|| fs::read(path))
        .wrap_err_with(|| format!("source {}", path.display()))?;
    if crate::resource::kind(path, &options.css) != crate::resource::ResourceKind::Script {
        return Ok(Prepared::Resource { input, format });
    }
    if crate::transpile::required(path) {
        let config = ParserConfig::for_path(path, format)?;
        if let Some(previous) = previous.filter(|previous| {
            previous.input == input
                && previous.transpiled.as_ref().is_some_and(|transpiled| {
                    transpiled.config == config
                        && (!options.output.source_maps || transpiled.source_map.is_some())
                })
        }) {
            let parser_path = path.with_extension(if previous.record.config().commonjs() {
                "cjs"
            } else {
                "mjs"
            });
            let mut loaded = load(
                path,
                input,
                previous.text.clone(),
                &parser_path,
                format,
                Some(previous),
                options.cache,
                declarations,
            )?;
            if loaded.source.is_some() {
                loaded.transpiled = previous.transpiled.clone();
            }
            return Ok(Prepared::Script(loaded));
        }
        let source = std::str::from_utf8(&input)
            .wrap_err_with(|| format!("{}: source is not UTF-8", path.display()))?;
        let compiled =
            tracing::info_span!("prepare.transpile", bytes = input.len()).in_scope(|| {
                crate::transpile::compile(path, source, format, options.output.source_maps)
            })?;
        let mut loaded = load(
            path,
            input,
            compiled.source,
            &compiled.parser_path,
            format,
            previous,
            options.cache,
            declarations,
        )?;
        loaded.transpiled = Some(Transpiled {
            config,
            source_map: compiled.source_map,
            source_map_hint: compiled.source_map_hint,
        });
        return Ok(Prepared::Script(loaded));
    }
    let wrapper_format = match options.resolver.target_env {
        TargetEnv::Browser => ModuleFormat::Esm,
        TargetEnv::Node => ModuleFormat::CommonJs,
    };
    let text = crate::resource::prepare(path, &input, base, wrapper_format)?;
    Ok(Prepared::Script(load(
        path,
        input,
        text,
        path,
        format,
        previous,
        options.cache,
        declarations,
    )?))
}

pub(super) fn load(
    path: &Path,
    input: Vec<u8>,
    text: String,
    parser_path: &Path,
    package: ModuleFormat,
    previous: Option<&Source>,
    cache: CacheMode,
    declarations: &PreparedDeclarations,
) -> eyre::Result<Loaded> {
    let _span = tracing::info_span!("prepare.load", bytes = input.len()).entered();
    let config = ParserConfig::for_path(parser_path, package)?;
    let previous = tracing::info_span!("prepare.memory_check").in_scope(|| {
        previous.filter(|s| {
            s.input == input
                && s.text == text
                && config.accepts(s.record.config())
                && s.record.matches_prepared_declarations(declarations)
        })
    });
    if previous.is_some() {
        return Ok(Loaded {
            source: None,
            status: None,
            warning: None,
            transpiled: None,
        });
    }
    let loaded = segment_cache::load_prepared_with_declarations(
        &logger::StdioLogger::new(),
        path,
        segment_cache::PreparedSource {
            source: &text,
            original: &input,
            parser_path,
            package,
        },
        cache,
        declarations,
    )
    .wrap_err_with(|| format!("loading source facts for {}", path.display()))?;
    let warning = if let CacheWrite::Failed(message) = loaded.write {
        Some(format!("{}: cache write: {message}", path.display()))
    } else {
        None
    };
    Ok(Loaded {
        source: Some(Source {
            css_identity: None,
            input,
            text: loaded.source,
            record: loaded.record,
            transpiled: None,
        }),
        status: Some(loaded.status),
        warning,
        transpiled: None,
    })
}
