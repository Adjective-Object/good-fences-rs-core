use parking_lot::RwLock;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use cache_flatbuffers::{resolution::Key, resolution_wire as wire};
use ftree_cache::inputs::{InputKind, InputProof, InputSnapshot, PathInput};

use super::*;
use crate::{
    lockfile::ProjectLockfile,
    resolution_cache::{CheckedResolution, Request, Roots, Store, Target},
};

type RequestKey = (u8, String, String);
type MemoKey = ([u8; 32], [u8; 32], [u8; 32], PathBuf, u8, String);

#[cfg(test)]
thread_local! {
    pub(super) static REQUEST_OBSERVER: std::cell::RefCell<Option<Arc<dyn Fn() + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
enum Proof {
    Fresh(InputProof),
    Disk(Arc<CheckedResolution>, RequestKey),
}

impl Proof {
    fn file_routes(&self, target: &Path) -> eyre::Result<Vec<PathBuf>> {
        match self {
            Self::Fresh(inputs) => Ok(inputs
                .inputs()
                .iter()
                .filter_map(|input| {
                    if !matches!(input.kind(), InputKind::File | InputKind::Directory) {
                        return None;
                    }
                    let relative = target.strip_prefix(input.canonical()?).ok()?;
                    Some(if relative.as_os_str().is_empty() {
                        input.path().into()
                    } else {
                        input.path().join(relative)
                    })
                })
                .collect()),
            Self::Disk(record, (mode, importer, specifier)) => {
                record.file_routes(wire::Mode(*mode), importer, specifier, target)
            }
        }
    }

    fn validate_in(&self, snapshot: &InputSnapshot) -> eyre::Result<()> {
        match self {
            Self::Fresh(inputs) => inputs.validate_in(snapshot),
            Self::Disk(record, (mode, importer, specifier)) => {
                record.revalidate_request_in(wire::Mode(*mode), importer, specifier, snapshot)
            }
        }
    }
}

pub(super) struct Admitting<'a> {
    resolver: BuildResolver,
    inventory: &'a Inventory,
}

impl std::ops::Deref for Admitting<'_> {
    type Target = BuildResolver;

    fn deref(&self) -> &Self::Target {
        &self.resolver
    }
}

impl DependencyResolver for Admitting<'_> {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        self.resolver.validate_speculation(base, specifier, mode)?;
        let target = self.resolver.resolve_dependency(base, specifier, mode)?;
        self.resolver.mark_demanded(base, specifier, mode)?;
        if let ResolvedDependency::File(path) = &target {
            let mode_id = if mode == ResolutionMode::Import { 0 } else { 1 };
            let package = self.resolver.package(base)?;
            let key = self.resolver.packages.read()[&package].key;
            let memo_key = (
                key.policy,
                key.instance,
                key.dependencies,
                base.into(),
                mode_id,
                specifier.into(),
            );
            let proof = self
                .resolver
                .used
                .read()
                .get(&memo_key)
                .map(|memo| memo.proof.clone())
                .expect("resolved request proof");
            let mut routes = proof.file_routes(path)?;
            // A package resolved from a canonical linked importer can also have
            // a named project installation. Check that exact package edge only;
            // never search node_modules or authorize an unlinked ancestor tree.
            let options = &self.resolver.inner_options;
            let request = options
                .alias
                .get(specifier)
                .map(String::as_str)
                .unwrap_or(specifier);
            if !request.starts_with(['.', '/', '\\']) && !Path::new(request).is_absolute() {
                let mut parts = request.split('/');
                if let Some(first) = parts.next() {
                    let name = if first.starts_with('@') {
                        parts.next().map(|second| format!("{first}/{second}"))
                    } else {
                        Some(first.to_owned())
                    };
                    if let Some(name) = name {
                        let alias = self.resolver.root.join("node_modules").join(name);
                        if let Some(canonical) =
                            self.resolver.inner.snapshot().capture(&alias)?.canonical()
                        {
                            if let Ok(relative) = path.strip_prefix(canonical) {
                                routes.push(if relative.as_os_str().is_empty() {
                                    alias
                                } else {
                                    alias.join(relative)
                                });
                            }
                        }
                    }
                }
            }
            routes.sort();
            routes.dedup();
            // Prefer named routes; a canonical-only observation is not itself
            // authority to read outside the checked input trees.
            let mut admitted = self.inventory.contains(path);
            for route in routes {
                if self.inventory.can_admit(&route) {
                    ensure!(
                        self.inventory.admit(&route).wrap_err_with(|| format!(
                            "{}: resolving {specifier:?}",
                            base.display()
                        ))? == *path,
                        "resolved route changed"
                    );
                    admitted = true;
                }
            }
            ensure!(admitted, "{}: resolving {specifier:?}: source outside reachable build inventory (or reserved/excluded): {}", base.display(), path.display());
        }
        Ok(target)
    }
}

#[derive(Clone)]
struct Memo {
    target: ResolvedDependency,
    proof: Arc<Proof>,
    cache_hit: bool,
}

#[derive(Clone, Default)]
pub(super) struct Memory(Arc<RwLock<BTreeMap<MemoKey, Memo>>>);

impl Memory {
    pub(super) fn retain_used(&self, script: &BuildResolver, sass: &BuildResolver) {
        let script = script.used.read();
        let sass = sass.used.read();
        self.0
            .write()
            .retain(|key, _| script.contains_key(key) || sass.contains_key(key));
    }
}

/// One parser invocation shared by ordinary and stylesheet resolver policies.
pub(super) struct Lockfile {
    root: PathBuf,
    selected: Option<PathBuf>,
    input: Option<PathInput>,
    graph: Option<ProjectLockfile>,
}

impl Lockfile {
    pub(super) fn read(
        root: &Path,
        warnings: &mut Vec<String>,
        execution: crate::execution::ExecutionOptions,
    ) -> eyre::Result<Arc<Self>> {
        let _span = tracing::info_span!("resolve.lockfile").entered();
        let (_, selected) = aube_lockfile::active_lockfile_path_selecting(root, None);
        let capture = selected
            .as_ref()
            .map(|path| {
                let mut input = PathInput::capture(path)?;
                input.seal_content(&fs::read(path)?);
                Ok::<_, eyre::Report>(input)
            })
            .transpose();
        let input = match capture {
            Ok(input) => input,
            Err(error) => {
                warnings.push(format!(
                    "resolution cache lockfile unavailable; using scoped proofs: {error:#}"
                ));
                None
            }
        };
        let parsed = if selected.is_some() && input.is_none() {
            Ok(None)
        } else {
            ProjectLockfile::read_with_execution(root, None, execution)
        };
        let graph = match parsed {
            Ok(graph) => graph,
            Err(error) => {
                warnings.push(format!(
                    "resolution cache lockfile unavailable; using scoped proofs: {error:#}"
                ));
                None
            }
        };
        let snapshot = Arc::new(Self {
            root: root.into(),
            selected,
            input,
            graph,
        });
        snapshot.recheck()?;
        Ok(snapshot)
    }

    fn recheck(&self) -> eyre::Result<()> {
        ensure!(
            aube_lockfile::active_lockfile_path_selecting(&self.root, None).1 == self.selected,
            "lockfile selection changed during build"
        );
        if let Some(input) = &self.input {
            input.validate()?;
        }
        if let Some(graph) = &self.graph {
            graph.recheck()?;
            ensure!(
                self.selected
                    .as_ref()
                    .is_some_and(|path| path.canonicalize().is_ok_and(|p| p == graph.path)),
                "lockfile selection identity changed"
            );
        }
        Ok(())
    }
}

struct Package {
    roots: Roots,
    key: Key,
    store: Store,
    loaded: Option<Arc<CheckedResolution>>,
    fresh: BTreeMap<RequestKey, Memo>,
}

/// The source-edge map is derived state, never a resolution authority. Each
/// build consults actual requests and retains their original proof until return.
pub(super) struct BuildResolver {
    #[cfg(test)]
    observer: Option<Arc<dyn Fn() + Send + Sync>>,
    root: PathBuf,
    inner: StructuralResolver,
    inner_options: NodeModulesResolverOptions,
    policy: [u8; 32],
    mode: CacheMode,
    lockfile: Arc<Lockfile>,
    memory: Memory,
    used: RwLock<BTreeMap<MemoKey, Memo>>,
    demanded: RwLock<BTreeSet<MemoKey>>,
    packages: RwLock<BTreeMap<PathBuf, Package>>,
    bases: RwLock<BTreeMap<PathBuf, PathBuf>>,
    package_inputs: RwLock<Vec<PathInput>>,
    last_validation: RwLock<Option<InputSnapshot>>,
    warnings: RwLock<Vec<(PathBuf, String)>>,
    calls: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl BuildResolver {
    fn validate_speculation(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<()> {
        let package = self.package(base)?;
        let key = self.packages.read()[&package].key;
        let request = (
            key.policy,
            key.instance,
            key.dependencies,
            base.into(),
            if mode == ResolutionMode::Import { 0 } else { 1 },
            specifier.into(),
        );
        if !self.demanded.read().contains(&request) {
            let memo = self.used.read().get(&request).cloned();
            if let Some(memo) = memo {
                memo.proof
                    .validate_in(self.inner.snapshot())
                    .wrap_err_with(|| {
                        format!(
                            "revalidating speculative resolution of {specifier:?} from {}",
                            base.display()
                        )
                    })?;
            }
        }
        Ok(())
    }

    pub(super) fn speculate(&self, path: &Path, record: &CacheRecord) {
        for dependency in record.module().facts().dependencies() {
            if dependency.type_only() || dependency.pattern().is_some() {
                continue;
            }
            let mode = if dependency.kind() == DependencyKind::Require {
                ResolutionMode::Require
            } else {
                ResolutionMode::Import
            };
            let _ = self.resolve_dependency(path, &dependency.specifier(), mode);
        }
    }

    fn mark_demanded(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<()> {
        let package = self.package(base)?;
        let key = self.packages.read()[&package].key;
        self.demanded.write().insert((
            key.policy,
            key.instance,
            key.dependencies,
            base.into(),
            if mode == ResolutionMode::Import { 0 } else { 1 },
            specifier.into(),
        ));
        Ok(())
    }

    pub(super) fn retain_precise(&self) {
        let mut used = self.used.write();
        let demanded = self.demanded.read();
        used.retain(|key, _| demanded.contains(key));
        let proofs: BTreeSet<_> = used
            .values()
            .map(|memo| Arc::as_ptr(&memo.proof) as usize)
            .collect();
        let bases = self.bases.read();
        let packages: BTreeSet<_> = used
            .keys()
            .filter_map(|key| bases.get(key.3.parent()?).cloned())
            .collect();
        let ancestry: BTreeSet<_> = used
            .keys()
            .flat_map(|key| {
                key.3
                    .parent()
                    .into_iter()
                    .flat_map(Path::ancestors)
                    .map(Path::to_owned)
            })
            .collect();
        self.packages.write().retain(|path, package| {
            package
                .fresh
                .retain(|_, memo| proofs.contains(&(Arc::as_ptr(&memo.proof) as usize)));
            packages.contains(path)
        });
        self.package_inputs.write().retain(|input| {
            input
                .path()
                .parent()
                .is_some_and(|parent| ancestry.contains(parent))
        });
        self.warnings
            .write()
            .retain(|(path, _)| packages.contains(path));
    }

    pub(super) fn admitting(mut self, inventory: &Inventory) -> Admitting<'_> {
        self.inner.set_snapshot(inventory.snapshot().clone());
        Admitting {
            resolver: self,
            inventory,
        }
    }

    pub(super) fn options(options: &BuildOptions) -> NodeModulesResolverOptions {
        let mut resolver = options.resolver.clone();
        if [TargetEnv::Node, TargetEnv::Browser]
            .into_iter()
            .any(|target| {
                resolver.export_conditions
                    == NodeModulesResolverOptions::default_for_env(target).export_conditions
            })
        {
            resolver.export_conditions =
                NodeModulesResolverOptions::default_for_env(resolver.target_env).export_conditions;
        }
        resolver
    }

    pub(super) fn policy(options: &BuildOptions) -> [u8; 32] {
        static IMPLEMENTATION: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
        let implementation = IMPLEMENTATION.get_or_init(|| {
            hash(
                concat!(
                    include_str!("resolver.rs"),
                    include_str!("../../../import_resolver/src/structural.rs"),
                    include_str!("../../../import_resolver/src/resolve.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/caching.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/mod.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/combined_resolver.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/common.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/internal_resolver.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/pkgjson_rewrites.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/util/mod.rs"),
                    include_str!("../../../import_resolver/src/node_resolver/util/pkgname.rs"),
                    include_str!("../../../packagejson/src/packagejson.rs"),
                    include_str!("../../../packagejson/src/exported_path.rs"),
                    include_str!("../../../packagejson_exports/src/lib.rs"),
                    include_str!("../../../ftree_cache/src/inputs.rs"),
                    include_str!("../../../ftree_cache/src/inputs/snapshot.rs"),
                    include_str!("../../../ftree_cache/src/context_data.rs")
                )
                .as_bytes(),
            )
        });
        let resolver = Self::options(options);
        let mut aliases: Vec<_> = resolver.alias.iter().collect();
        aliases.sort();
        hash(
            format!(
                "scoped-resolution-v1:{}:{}:{:?}:{:?}:{aliases:?}:{:?}:{:?}:{}:{}:{:?}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                resolver.target_env,
                resolver.extensions,
                resolver.export_conditions,
                options.externals,
                resolver.ignore_node_modules,
                resolver.preserve_symlinks,
                implementation
            )
            .as_bytes(),
        )
    }

    pub(super) fn new(
        root: &Path,
        options: &BuildOptions,
        lockfile: Arc<Lockfile>,
        memory: Memory,
    ) -> Self {
        Self {
            #[cfg(test)]
            observer: REQUEST_OBSERVER.with(|observer| observer.borrow().clone()),
            root: root.into(),
            inner: StructuralResolver::new(
                root.into(),
                Self::options(options),
                options.externals.clone(),
            ),
            inner_options: Self::options(options),
            policy: Self::policy(options),
            mode: options.cache,
            lockfile,
            memory,
            used: RwLock::new(BTreeMap::new()),
            demanded: RwLock::new(BTreeSet::new()),
            packages: RwLock::new(BTreeMap::new()),
            bases: RwLock::new(BTreeMap::new()),
            package_inputs: RwLock::new(Vec::new()),
            last_validation: RwLock::new(None),
            warnings: RwLock::new(Vec::new()),
            calls: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    fn package(&self, base: &Path) -> eyre::Result<PathBuf> {
        let directory = base
            .parent()
            .ok_or_else(|| eyre::eyre!("resolution base has no parent"))?;
        if let Some(package) = self.bases.read().get(directory) {
            return Ok(package.clone());
        }
        let fallback = if base.starts_with(&self.root) {
            self.root.clone()
        } else {
            directory.into()
        };
        let mut package = fallback;
        let mut manifest_bytes = None;
        for directory in directory.ancestors() {
            // Reuse the original sealed package observation, never restat it
            // and attribute new bytes to a previously computed key.
            if self.packages.read().contains_key(directory) {
                package = directory.into();
                break;
            }
            let path = directory.join("package.json");
            let mut input = self.inner.snapshot().capture(&path)?;
            if input.kind() == InputKind::File {
                let (sealed, bytes) = self.inner.snapshot().read(&path)?;
                input = sealed;
                package = directory.into();
                manifest_bytes = bytes;
                self.package_inputs.write().push(input);
                break;
            }
            self.package_inputs.write().push(input);
            if directory == self.root {
                break;
            }
        }
        if !self.packages.read().contains_key(&package) {
            let manifest = manifest_bytes
                .as_deref()
                .map(serde_json::from_slice::<serde_json::Value>)
                .transpose()?
                .unwrap_or_default();
            let locked = self
                .lockfile
                .graph
                .as_ref()
                .and_then(|graph| graph.physical_package(&self.root, &package, &manifest));
            // Incomplete graphs (notably Berry installs without locators) remain
            // usable through the full actual-request proof, not guessed peers.
            let identity = package.strip_prefix(&self.root).unwrap_or(&package);
            let instance = hash(&serde_json::to_vec(&(
                "physical-package-v1",
                identity.as_os_str().as_encoded_bytes(),
                manifest_bytes.as_deref(),
                self.policy,
                locked.map(|p| (p.instance, p.mutable, p.complete)),
            ))?);
            let dependencies = hash(&serde_json::to_vec(&(
                "named-dependencies-v1",
                &manifest["dependencies"],
                &manifest["optionalDependencies"],
                &manifest["peerDependencies"],
                locked.map(|p| p.dependencies),
                self.lockfile
                    .graph
                    .as_ref()
                    .map(|g| format!("{:?}", g.kind)),
            ))?);
            let key = Key {
                policy: self.policy,
                instance,
                dependencies,
            };
            let roots = Roots::new_in(&self.root, &package, self.inner.snapshot())?;
            let store = Store::new(roots.clone(), key, self.mode);
            let load = store.load_scoped_in(self.inner.snapshot());
            if let CacheStatus::Miss(
                reason @ (segment_cache::MissReason::InvalidRecord(_)
                | segment_cache::MissReason::ReadFailure(_)),
            ) = &load.status
            {
                self.warnings.write().push((
                    package.clone(),
                    format!("{}: resolution cache miss: {reason:?}", package.display()),
                ));
            }
            self.packages
                .write()
                .entry(package.clone())
                .or_insert_with(|| Package {
                    roots,
                    key,
                    store,
                    loaded: load.record.map(Arc::new),
                    fresh: BTreeMap::new(),
                });
        }
        self.bases.write().insert(directory.into(), package.clone());
        Ok(package)
    }

    pub(super) fn recheck(&self) -> eyre::Result<()> {
        let _span = tracing::info_span!(
            "resolve.recheck",
            packages = self.packages.read().len(),
            requests = self.used.read().len()
        )
        .entered();
        self.lockfile.recheck()?;
        let snapshot = InputSnapshot::default();
        for input in self.package_inputs.read().iter() {
            snapshot.validate(input)?;
        }
        for memo in self.used.read().values() {
            memo.proof.validate_in(&snapshot)?;
        }
        *self.last_validation.write() = Some(snapshot);
        Ok(())
    }

    /// Called only after all build inputs have passed final validation.
    pub(super) fn finish(&self, counters: &mut Counters, warnings: &mut Vec<String>) {
        let used = self.used.read();
        let hits = used.values().filter(|memo| memo.cache_hit).count();
        counters.resolutions += used.len() - hits;
        counters.resolution_cache_hits += hits;
        counters.resolution_cache_misses += used.len() - hits;
        warnings.extend(
            self.warnings
                .read()
                .iter()
                .map(|(_, warning)| warning.clone()),
        );
        if self.mode != CacheMode::ReadWrite {
            return;
        }
        let snapshot = self.last_validation.read().clone().unwrap_or_default();
        for (path, package) in self.packages.read().iter() {
            if package.fresh.is_empty() {
                continue;
            }
            let mut records: Vec<_> = package.loaded.iter().map(|r| r.as_ref().clone()).collect();
            for ((mode, importer, specifier), memo) in &package.fresh {
                let Proof::Fresh(inputs) = memo.proof.as_ref() else {
                    continue;
                };
                let target = match &memo.target {
                    ResolvedDependency::File(path) => Target::File(path),
                    ResolvedDependency::External(name) => Target::External(name),
                };
                let encoded = CheckedResolution::encode_in(
                    package.roots.clone(),
                    package.key,
                    &[Request {
                        mode: wire::Mode(*mode),
                        importer,
                        specifier,
                        target,
                        inputs,
                    }],
                    &snapshot,
                );
                match encoded {
                    Ok(record) => records.push(record),
                    Err(error) => warnings.push(format!(
                        "{}: resolution cache request not stored: {error:#}",
                        path.display()
                    )),
                }
            }
            if records.is_empty() {
                continue;
            }
            match CheckedResolution::aggregate_in(
                package.roots.clone(),
                package.key,
                &records,
                &snapshot,
            ) {
                Ok(record) => {
                    if let CacheWrite::Failed(error) = package.store.save_in(&record, &snapshot) {
                        warnings.push(format!(
                            "{}: resolution cache write failed: {error}",
                            path.display()
                        ));
                    }
                }
                Err(error) => warnings.push(format!(
                    "{}: resolution cache aggregate not stored: {error:#}",
                    path.display()
                )),
            }
        }
    }
}

impl Drop for BuildResolver {
    fn drop(&mut self) {
        // Counts survive returned build errors, without logging requests or paths.
        let _span = tracing::info_span!(
            "resolve.counts",
            resolutions = self.calls.load(Ordering::Relaxed),
            hits = self.hits.load(Ordering::Relaxed),
            misses = self.misses.load(Ordering::Relaxed),
            packages = self.packages.get_mut().len()
        )
        .entered();
    }
}

impl DependencyResolver for BuildResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        let _span = tracing::info_span!("resolve.request").entered();
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            observer();
        }
        let package_path = self.package(base)?;
        let mode_id = if mode == ResolutionMode::Import { 0 } else { 1 };
        let importer = base
            .strip_prefix(&package_path)?
            .components()
            .map(|c| c.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join("/"));
        let cacheable = importer.is_some();
        let importer = importer.unwrap_or_default();
        let request_key = (mode_id, importer, specifier.to_owned());
        let key = self.packages.read()[&package_path].key;
        let memo_key = (
            key.policy,
            key.instance,
            key.dependencies,
            base.into(),
            mode_id,
            specifier.into(),
        );
        if let Some(memo) = self.used.read().get(&memo_key) {
            return Ok(memo.target.clone());
        }
        let memory = self.memory.0.read().get(&memo_key).cloned();
        let mut memo = memory.filter(|memo| memo.proof.validate_in(self.inner.snapshot()).is_ok());
        if memo.is_none() && cacheable {
            let loaded = self.packages.read()[&package_path].loaded.clone();
            if let Some(record) = loaded {
                if let Some(request) = record.find(wire::Mode(mode_id), &request_key.1, specifier) {
                    record.revalidate_request_in(
                        wire::Mode(mode_id),
                        &request_key.1,
                        specifier,
                        self.inner.snapshot(),
                    )?;
                    let target = match request.target().external() {
                        Some(name) => ResolvedDependency::External(name.into()),
                        None => ResolvedDependency::File(
                            record
                                .file_target(wire::Mode(mode_id), &request_key.1, specifier)?
                                .expect("checked file target"),
                        ),
                    };
                    memo = Some(Memo {
                        target,
                        proof: Arc::new(Proof::Disk(record, request_key.clone())),
                        cache_hit: true,
                    });
                }
            }
        }
        let memo = match memo {
            Some(mut memo) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                memo.cache_hit = true;
                memo
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                self.calls.fetch_add(1, Ordering::Relaxed);
                let tracked = self
                    .inner
                    .resolve_dependency_with_inputs(base, specifier, mode)?;
                tracked.inputs.validate_in(self.inner.snapshot())?;
                Memo {
                    target: tracked.dependency,
                    proof: Arc::new(Proof::Fresh(tracked.inputs)),
                    cache_hit: false,
                }
            }
        };
        if (self.hits.load(Ordering::Relaxed) + self.misses.load(Ordering::Relaxed))
            .is_multiple_of(1024)
        {
            let _span = tracing::info_span!(
                "resolve.progress",
                resolutions = self.calls.load(Ordering::Relaxed),
                hits = self.hits.load(Ordering::Relaxed),
                misses = self.misses.load(Ordering::Relaxed),
                packages = self.packages.read().len()
            )
            .entered();
        }
        if cacheable && matches!(memo.proof.as_ref(), Proof::Fresh(_)) {
            let mut packages = self.packages.write();
            let package = packages.get_mut(&package_path).expect("package");
            if package.loaded.as_ref().is_none_or(|record| {
                record
                    .find(wire::Mode(mode_id), &request_key.1, specifier)
                    .is_none()
            }) {
                package.fresh.insert(request_key, memo.clone());
            }
        }
        self.used.write().insert(memo_key.clone(), memo.clone());
        self.memory.0.write().insert(memo_key, memo.clone());
        Ok(memo.target)
    }
}
