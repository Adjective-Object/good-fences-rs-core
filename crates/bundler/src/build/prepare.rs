//! Bounded source preparation and import-resolution jobs.
use super::*;

#[cfg(test)]
type SourceObserver = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;
#[cfg(test)]
type FrontierObserver = std::sync::Arc<dyn Fn(&Path, usize) + Send + Sync>;

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_PREPARE: std::cell::RefCell<Option<SourceObserver>> =
        const { std::cell::RefCell::new(None) };
    pub(super) static AFTER_DEPENDENCIES: std::cell::RefCell<Option<SourceObserver>> =
        const { std::cell::RefCell::new(None) };
    pub(super) static EARLY_CHILDREN: std::cell::RefCell<Option<FrontierObserver>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) enum Job {
    Prepare(PathBuf),
    PrepareEarly(PathBuf),
    Speculate(PathBuf),
    ResolveSpeculative {
        path: PathBuf,
    },
    Resolve {
        path: PathBuf,
        loaded: Loaded,
        css_identity: Option<PathBuf>,
    },
    ResolveEarly {
        path: PathBuf,
        dependencies: Dependencies,
    },
}

impl Job {
    pub(super) fn priority(&self) -> usize {
        match self {
            Self::Resolve { .. } | Self::ResolveEarly { .. } => 0,
            Self::Prepare(_) | Self::PrepareEarly(_) => 1,
            Self::ResolveSpeculative { .. } => 2,
            Self::Speculate(_) => 3,
        }
    }

    pub(super) fn preparation(&self) -> bool {
        matches!(
            self,
            Self::Prepare(_) | Self::PrepareEarly(_) | Self::Speculate(_)
        )
    }
}

/// Provisional syntax or a checked borrowed owner, never solved cache state.
#[derive(Clone)]
pub(super) enum Dependencies {
    Early(std::sync::Arc<[ast_segmenter::owned::Dependency]>),
    Checked(CacheRecord),
}

impl Dependencies {
    pub(super) fn iter(
        &self,
    ) -> ast_segmenter::views::Sequence<'_, ast_segmenter::views::Dependency<'_>> {
        match self {
            Self::Early(dependencies) => ast_segmenter::views::Sequence::Owned(dependencies),
            Self::Checked(record) => record.module().facts().dependencies(),
        }
    }
}

pub(super) struct Early {
    pub path: PathBuf,
    pub dependencies: Dependencies,
    pub admission: policy::Admission,
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
    claims: parking_lot::Mutex<BTreeMap<PathBuf, bool>>,
    ready: parking_lot::Mutex<BTreeMap<PathBuf, Completion>>,
    limit: usize,
    parses: std::sync::atomic::AtomicUsize,
    promoted: std::sync::atomic::AtomicUsize,
}

impl Speculation {
    pub(super) fn new(paths: &BTreeSet<PathBuf>, workers: usize) -> Self {
        Self {
            claims: parking_lot::Mutex::new(
                paths.iter().map(|path| (path.clone(), false)).collect(),
            ),
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
        let mut ready = self.ready.lock();
        if let Some(ready) = ready.remove(path) {
            return Claim::Ready(ready);
        }
        let mut claims = self.claims.lock();
        let claimed = claims.entry(path.into()).or_default();
        if std::mem::replace(claimed, true) {
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
            *self.claims.lock().get_mut(&path).expect("claimed source") = false;
        }
    }

    pub(super) fn record(&self, path: &Path, previous: Option<&Source>) -> Option<CacheRecord> {
        let ready = self.ready.lock();
        let (Ok(Prepared::Script(loaded)), _, _) = ready.get(path)? else {
            return None;
        };
        loaded
            .source
            .as_ref()
            .or(previous)
            .map(|source| source.record.clone())
    }
}

pub(super) enum Prepared {
    Dependencies {
        dependencies: Dependencies,
        admission: policy::Admission,
    },
    EarlyResolved {
        resolutions: Resolutions,
    },
    Script(Loaded),
    Resource {
        input: Vec<u8>,
        format: ModuleFormat,
    },
    Resolved {
        loaded: Loaded,
        css_identity: Option<PathBuf>,
        resolutions: Resolutions,
    },
}

#[derive(Default)]
pub(super) struct Resolutions {
    pub targets: HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>,
    pub errors: Vec<eyre::Report>,
}

impl Resolutions {
    pub(super) fn report_errors(
        &mut self,
        path: &Path,
        errors: &mut BTreeMap<PathBuf, Vec<eyre::Report>>,
    ) {
        for error in self.errors.drain(..) {
            tracing::error!(target: "bundler::diagnostic", "{error:#}");
            errors.entry(path.to_owned()).or_default().push(error);
        }
    }
}

pub(super) fn resolve(
    path: &Path,
    dependencies: &Dependencies,
    inventory: &Inventory,
    options: &BuildOptions,
    resolver: &impl DependencyResolver,
) -> Resolutions {
    let _span = tracing::info_span!("build.resolve").entered();
    let mut resolved = Resolutions::default();
    for dependency in dependencies.iter() {
        if dependency.type_only() {
            continue;
        }
        let _dependency_span = tracing::info_span!(
            "resolve.dependency",
            context = dependency.pattern().is_some(),
            require = dependency.kind() == DependencyKind::Require
        )
        .entered();
        let result = (|| -> eyre::Result<DependencyTarget> {
            let target = if dependency.kind() == DependencyKind::ImportMetaEnv {
                DependencyTarget::File(env::path(inventory.root()))
            } else if dependency.pattern().is_some() {
                DependencyTarget::Context(
                    crate::context::resolve(
                        inventory.root(),
                        path,
                        dependency,
                        inventory.candidates(
                            path,
                            dependencies,
                            dependency,
                            &options.resolver.extensions,
                        )?,
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
            if dependency.kind() == DependencyKind::Url {
                worker_entries::target(path, dependency, Some(&target), options)?;
            }
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
            Ok(target)
        })();
        match result {
            Ok(target) => {
                resolved.targets.insert(dependency.id(), target);
            }
            Err(error) => resolved.errors.push(error),
        }
    }
    resolved
}

pub(super) struct Loaded {
    pub admission: policy::Admission,
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
    dependencies: &dyn Fn(&[ast_segmenter::owned::Dependency], policy::Admission),
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
            previous.text.matches_input(&input)
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
                previous.text.text(path)?.to_string(),
                &parser_path,
                format,
                Some(previous),
                options.cache,
                (inventory.root(), &options.policy_allowlist),
                declarations,
                dependencies,
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
            (inventory.root(), &options.policy_allowlist),
            declarations,
            dependencies,
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
        (inventory.root(), &options.policy_allowlist),
        declarations,
        dependencies,
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
    policy: (&Path, &[PolicyAllowance]),
    declarations: &PreparedDeclarations,
    dependencies: &dyn Fn(&[ast_segmenter::owned::Dependency], policy::Admission),
) -> eyre::Result<Loaded> {
    let _span = tracing::info_span!("prepare.load", bytes = input.len()).entered();
    let config = ParserConfig::for_path(parser_path, package)?;
    let previous = tracing::info_span!("prepare.memory_check").in_scope(|| {
        previous.filter(|s| {
            s.text.matches(&input, &text)
                && config.accepts(s.record.config())
                && s.record.matches_prepared_declarations(declarations)
        })
    });
    if let Some(previous) = previous {
        return Ok(Loaded {
            admission: policy::Admission::check(
                policy.0,
                path,
                &text,
                previous.record.module().facts().policy_violations(),
                policy.1,
            ),
            source: None,
            status: None,
            warning: None,
            transpiled: None,
        });
    }
    let loaded = segment_cache::load_prepared_reporting_policies(
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
        |deps, violations| {
            dependencies(
                deps,
                policy::Admission::check(
                    policy.0,
                    path,
                    &text,
                    violations.iter().copied(),
                    policy.1,
                ),
            )
        },
    )
    .wrap_err_with(|| format!("loading source facts for {}", path.display()))?;
    let warning = if let CacheWrite::Failed(message) = loaded.write {
        Some(format!("{}: cache write: {message}", path.display()))
    } else {
        None
    };
    Ok(Loaded {
        admission: policy::Admission::check(
            policy.0,
            path,
            &text,
            loaded.record.module().facts().policy_violations(),
            policy.1,
        ),
        source: Some(Source {
            css_identity: None,
            text: crate::prepared_text::PreparedText::new(
                &input,
                &loaded.source,
                parser_path != path,
            )
            .wrap_err_with(|| format!("sealing prepared text for {}", path.display()))?,
            record: loaded.record,
            transpiled: None,
        }),
        status: Some(loaded.status),
        warning,
        transpiled: None,
    })
}
