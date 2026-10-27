use parking_lot::RwLock;
use std::hash::{Hash, Hasher};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use cache_flatbuffers::{resolution::Key, resolution_wire as wire};
use ftree_cache::inputs::{InputKind, InputProof, InputSnapshot, PathInput};

use super::scratch::{NativePath, Scratch};
use super::*;
use crate::{
    lockfile::ProjectLockfile,
    resolution_cache::{CheckedResolution, Request, Roots, Store, Target},
};

#[derive(Debug, PartialEq, Eq)]
struct MemoKey {
    policy: [u8; 32],
    instance: [u8; 32],
    dependencies: [u8; 32],
    base: PathBuf,
    mode: u8,
    specifier: String,
    hash: u64,
}

#[derive(Clone, Copy)]
struct RequestKey<'a> {
    policy: [u8; 32],
    instance: [u8; 32],
    dependencies: [u8; 32],
    base: NativePath<'a>,
    mode: u8,
    specifier: &'a str,
    hash: u64,
}

#[derive(Clone, Copy)]
struct Query<'a> {
    key: Key,
    base: &'a Path,
    mode: u8,
    specifier: &'a str,
    hash: u64,
}

impl Query<'_> {
    fn new<'a>(key: Key, base: &'a Path, mode: u8, specifier: &'a str) -> Query<'a> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (
            key.policy,
            key.instance,
            key.dependencies,
            base,
            mode,
            specifier,
        )
            .hash(&mut hash);
        Query {
            key,
            base,
            mode,
            specifier,
            hash: hash.finish(),
        }
    }

    fn allocate<'a>(self, scratch: &Scratch<'a>) -> RequestKey<'a> {
        RequestKey {
            policy: self.key.policy,
            instance: self.key.instance,
            dependencies: self.key.dependencies,
            base: scratch.path(self.base),
            mode: self.mode,
            specifier: scratch.text(self.specifier),
            hash: self.hash,
        }
    }
}

impl Hash for Query<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl Hash for MemoKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl Hash for RequestKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for RequestKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.policy == other.policy
            && self.instance == other.instance
            && self.dependencies == other.dependencies
            && self.mode == other.mode
            && self.specifier == other.specifier
            && (self.base == other.base || self.base.matches(&other.base.to_path_buf()))
    }
}
impl Eq for RequestKey<'_> {}

impl hashbrown::Equivalent<&RequestKey<'_>> for Query<'_> {
    fn equivalent(&self, key: &&RequestKey<'_>) -> bool {
        self.hash == key.hash
            && self.key.policy == key.policy
            && self.key.instance == key.instance
            && self.key.dependencies == key.dependencies
            && self.mode == key.mode
            && self.specifier == key.specifier
            && key.base.matches(self.base)
    }
}

impl hashbrown::Equivalent<Arc<MemoKey>> for Query<'_> {
    fn equivalent(&self, key: &Arc<MemoKey>) -> bool {
        self.hash == key.hash
            && self.key.policy == key.policy
            && self.key.instance == key.instance
            && self.key.dependencies == key.dependencies
            && self.mode == key.mode
            && self.specifier == key.specifier
            && self.base == key.base
    }
}

type RequestOrder<'a> = ([u8; 32], [u8; 32], [u8; 32], PathBuf, u8, &'a str);

impl<'a> RequestKey<'a> {
    fn owned(self) -> MemoKey {
        MemoKey {
            policy: self.policy,
            instance: self.instance,
            dependencies: self.dependencies,
            base: self.base.to_path_buf(),
            mode: self.mode,
            specifier: self.specifier.into(),
            hash: self.hash,
        }
    }

    fn sort_key(self) -> RequestOrder<'a> {
        // Path ordering compares normalized components, not arena-native units.
        (
            self.policy,
            self.instance,
            self.dependencies,
            self.base.to_path_buf(),
            self.mode,
            self.specifier,
        )
    }
}

impl MemoKey {
    fn query(&self) -> Query<'_> {
        Query {
            key: Key {
                policy: self.policy,
                instance: self.instance,
                dependencies: self.dependencies,
            },
            base: &self.base,
            mode: self.mode,
            specifier: &self.specifier,
            hash: self.hash,
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static REQUEST_OBSERVER: std::cell::RefCell<Option<Arc<dyn Fn() + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

enum Proof {
    Fresh(InputProof),
    Disk(Arc<CheckedResolution>, String),
}

struct Memo {
    key: Arc<MemoKey>,
    target: ResolvedDependency,
    proof: Arc<Proof>,
}

impl Memo {
    fn validate_in(&self, snapshot: &InputSnapshot) -> eyre::Result<()> {
        self.proof
            .validate_in(self.key.mode, &self.key.specifier, snapshot)
    }
}

impl Proof {
    fn file_routes(&self, mode: u8, specifier: &str, target: &Path) -> eyre::Result<Vec<PathBuf>> {
        match self {
            Proof::Fresh(inputs) => Ok(inputs
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
            Proof::Disk(record, importer) => {
                record.file_routes(wire::Mode(mode), importer, specifier, target)
            }
        }
    }

    fn validate_in(&self, mode: u8, specifier: &str, snapshot: &InputSnapshot) -> eyre::Result<()> {
        match self {
            Proof::Fresh(inputs) => inputs.validate_in(snapshot),
            Proof::Disk(record, importer) => {
                record.revalidate_request_in(wire::Mode(mode), importer, specifier, snapshot)
            }
        }
    }
}

pub(super) struct Admitting<'a, 'scratch, 'arena> {
    resolver: BuildResolver<'scratch, 'arena>,
    inventory: &'a Inventory,
}

impl<'scratch, 'arena> std::ops::Deref for Admitting<'_, 'scratch, 'arena> {
    type Target = BuildResolver<'scratch, 'arena>;

    fn deref(&self) -> &Self::Target {
        &self.resolver
    }
}

impl DependencyResolver for Admitting<'_, '_, '_> {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        let request = self.resolver.resolve_request(base, specifier, mode)?;
        self.resolver.demand(&request)?;
        let target = request.row.target.owned();
        if let ResolvedDependency::File(path) = &target {
            let mut routes = request.file_routes(path)?;
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

#[derive(Clone, Copy)]
enum TargetValue<'a> {
    File(NativePath<'a>),
    External(&'a str),
}

impl TargetValue<'_> {
    fn owned(self) -> ResolvedDependency {
        match self {
            Self::File(path) => ResolvedDependency::File(path.to_path_buf()),
            Self::External(name) => ResolvedDependency::External(name.into()),
        }
    }
}

#[derive(Clone, Copy)]
struct RequestRow<'a> {
    key: RequestKey<'a>,
    target: TargetValue<'a>,
}

struct CurrentRequest<'a> {
    row: &'a RequestRow<'a>,
    proof: Arc<Proof>,
    retained: Option<Arc<Memo>>,
    demanded: AtomicBool,
    cache_hit: bool,
}

#[derive(Clone, Default)]
pub(super) struct Memory(Arc<RwLock<hashbrown::HashMap<Arc<MemoKey>, Arc<Memo>>>>);

impl CurrentRequest<'_> {
    fn validate_in(&self, snapshot: &InputSnapshot) -> eyre::Result<()> {
        self.proof
            .validate_in(self.row.key.mode, self.row.key.specifier, snapshot)
    }

    fn file_routes(&self, target: &Path) -> eyre::Result<Vec<PathBuf>> {
        self.proof
            .file_routes(self.row.key.mode, self.row.key.specifier, target)
    }
}

impl Memory {
    pub(super) fn retain_used(
        &self,
        script: &BuildResolver<'_, '_>,
        sass: &BuildResolver<'_, '_>,
        css: &BuildResolver<'_, '_>,
    ) {
        let script = script.requests.read();
        let sass = sass.requests.read();
        let css = css.requests.read();
        self.0.write().retain(|key, _| {
            [
                script.get(&key.query()),
                sass.get(&key.query()),
                css.get(&key.query()),
            ]
            .into_iter()
            .flatten()
            .any(|request| request.demanded.load(Ordering::Relaxed))
        });
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

#[derive(Clone)]
struct Package<'a> {
    roots: Roots,
    key: Key,
    loaded: Option<Arc<CheckedResolution>>,
    fresh: Vec<Arc<CurrentRequest<'a>>>,
}

/// The source-edge map is derived state, never a resolution authority. Each
/// build consults actual requests and retains their original proof until return.
pub(super) struct BuildResolver<'scratch, 'a> {
    #[cfg(test)]
    observer: Option<Arc<dyn Fn() + Send + Sync>>,
    root: PathBuf,
    inner: StructuralResolver,
    inner_options: NodeModulesResolverOptions,
    policy: [u8; 32],
    mode: CacheMode,
    lockfile: Arc<Lockfile>,
    memory: Memory,
    scratch: &'scratch Scratch<'a>,
    requests: RwLock<hashbrown::HashMap<&'a RequestKey<'a>, Arc<CurrentRequest<'a>>>>,
    packages: RwLock<BTreeMap<PathBuf, Package<'a>>>,
    bases: RwLock<BTreeMap<PathBuf, PathBuf>>,
    package_inputs: RwLock<Vec<PathInput>>,
    last_validation: RwLock<Option<InputSnapshot>>,
    warnings: RwLock<Vec<(PathBuf, String)>>,
    calls: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl<'scratch, 'a> BuildResolver<'scratch, 'a> {
    fn demand(&self, request: &CurrentRequest<'_>) -> eyre::Result<()> {
        if !request.demanded.load(Ordering::Relaxed) {
            request
                .validate_in(self.inner.snapshot())
                .wrap_err_with(|| {
                    format!(
                        "revalidating speculative resolution of {:?} from {}",
                        request.row.key.specifier,
                        request.row.key.base.to_path_buf().display()
                    )
                })?;
            // Precise requests remain useful after failure, but speculative
            // requests must never escape this build through retained memory.
            let memo = request.retained.clone().unwrap_or_else(|| {
                Arc::new(Memo {
                    key: Arc::new(request.row.key.owned()),
                    target: request.row.target.owned(),
                    proof: request.proof.clone(),
                })
            });
            self.memory.0.write().insert(memo.key.clone(), memo);
            request.demanded.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(super) fn speculate(&self, path: &Path, record: &CacheRecord) {
        for dependency in record.module().facts().dependencies() {
            if dependency.type_only()
                || dependency.pattern().is_some()
                || dependency.kind() == DependencyKind::ImportMetaEnv
            {
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

    pub(super) fn retain_precise(&self) {
        let mut requests = self.requests.write();
        requests.retain(|_, request| request.demanded.load(Ordering::Relaxed));
        let bases = self.bases.read();
        let mut packages = BTreeSet::new();
        let mut ancestry = BTreeSet::new();
        for key in requests.keys() {
            let base = key.base.to_path_buf();
            if let Some(parent) = base.parent() {
                if let Some(package) = bases.get(parent) {
                    packages.insert(package.clone());
                }
                ancestry.extend(parent.ancestors().map(Path::to_owned));
            }
        }
        self.packages.write().retain(|path, package| {
            package
                .fresh
                .retain(|request| request.demanded.load(Ordering::Relaxed));
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

    pub(super) fn admitting(mut self, inventory: &Inventory) -> Admitting<'_, 'scratch, 'a> {
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
                    include_str!("scratch.rs"),
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
                (
                    resolver.extensions,
                    resolver.prefer_js_extension,
                    &options.loaders
                ),
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
        scratch: &'scratch Scratch<'a>,
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
            scratch,
            requests: RwLock::new(hashbrown::HashMap::new()),
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
                    loaded: load.record.map(Arc::new),
                    fresh: Vec::new(),
                });
        }
        self.bases.write().insert(directory.into(), package.clone());
        Ok(package)
    }

    pub(super) fn recheck(&self) -> eyre::Result<()> {
        let mut requests: Vec<_> = self.requests.read().values().cloned().collect();
        requests.sort_by_cached_key(|request| request.row.key.sort_key());
        let _span = tracing::info_span!(
            "resolve.recheck",
            packages = self.packages.read().len(),
            requests = requests.len()
        )
        .entered();
        self.lockfile.recheck()?;
        let snapshot = InputSnapshot::default();
        let inputs = self.package_inputs.read().clone();
        for input in &inputs {
            snapshot.validate(input)?;
        }
        for request in requests {
            request.validate_in(&snapshot)?;
        }
        *self.last_validation.write() = Some(snapshot);
        Ok(())
    }

    /// Called only after all build inputs have passed final validation.
    pub(super) fn finish(&self, counters: &mut Counters, warnings: &mut Vec<String>) {
        let requests = self.requests.read();
        let demanded = requests
            .values()
            .filter(|request| request.demanded.load(Ordering::Relaxed))
            .count();
        let hits = requests
            .values()
            .filter(|request| request.demanded.load(Ordering::Relaxed) && request.cache_hit)
            .count();
        drop(requests);
        counters.resolutions += demanded - hits;
        counters.resolution_cache_hits += hits;
        counters.resolution_cache_misses += demanded - hits;
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
        let packages: Vec<_> = self
            .packages
            .read()
            .iter()
            .filter(|(_, package)| !package.fresh.is_empty())
            .map(|(path, package)| (path.clone(), package.clone()))
            .collect();
        for (path, package) in packages {
            let mut records: Vec<_> = package.loaded.iter().map(|r| r.as_ref().clone()).collect();
            let mut fresh = package.fresh;
            fresh.sort_by_cached_key(|request| request.row.key.sort_key());
            for request in &fresh {
                if !request.demanded.load(Ordering::Relaxed) {
                    continue;
                }
                let Proof::Fresh(inputs) = request.proof.as_ref() else {
                    continue;
                };
                let base = request.row.key.base.to_path_buf();
                let importer = base
                    .strip_prefix(&path)
                    .expect("request package")
                    .components()
                    .map(|component| component.as_os_str().to_str().expect("cacheable importer"))
                    .collect::<Vec<_>>()
                    .join("/");
                let owned_target = request.row.target.owned();
                let target = match &owned_target {
                    ResolvedDependency::File(path) => Target::File(path),
                    ResolvedDependency::External(name) => Target::External(name),
                };
                let encoded = CheckedResolution::encode_in(
                    package.roots.clone(),
                    package.key,
                    &[Request {
                        mode: wire::Mode(request.row.key.mode),
                        importer: &importer,
                        specifier: request.row.key.specifier,
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
                    let store = Store::new(package.roots.clone(), package.key, self.mode);
                    if let CacheWrite::Failed(error) = store.save_in(&record, &snapshot) {
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

impl Drop for BuildResolver<'_, '_> {
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

impl<'scratch, 'a> BuildResolver<'scratch, 'a> {
    fn resolve_request(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<Arc<CurrentRequest<'a>>> {
        let _span = tracing::info_span!("resolve.request").entered();
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            observer();
        }
        let package_path = self.package(base)?;
        let mode_id = if mode == ResolutionMode::Import { 0 } else { 1 };
        let key = self.packages.read()[&package_path].key;
        let query = Query::new(key, base, mode_id, specifier);
        if let Some(request) = self.requests.read().get(&query) {
            return Ok(request.clone());
        }
        let importer = base
            .strip_prefix(&package_path)?
            .components()
            .map(|c| c.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join("/"));
        let cacheable = importer.is_some();
        let importer = importer.unwrap_or_default();
        let memory = self.memory.0.read().get(&query).cloned();
        let retained = memory.filter(|memo| memo.validate_in(self.inner.snapshot()).is_ok());
        let mut loaded = None;
        if retained.is_none() && cacheable {
            let record = self.packages.read()[&package_path].loaded.clone();
            if let Some(record) = record {
                if let Some(request) = record.find(wire::Mode(mode_id), &importer, specifier) {
                    record.revalidate_request_in(
                        wire::Mode(mode_id),
                        &importer,
                        specifier,
                        self.inner.snapshot(),
                    )?;
                    let target = match request.target().external() {
                        Some(name) => ResolvedDependency::External(name.into()),
                        None => ResolvedDependency::File(
                            record
                                .file_target(wire::Mode(mode_id), &importer, specifier)?
                                .expect("checked file target"),
                        ),
                    };
                    loaded = Some((target, Arc::new(Proof::Disk(record, importer.clone()))));
                }
            }
        }
        let cache_hit = retained.is_some() || loaded.is_some();
        if cache_hit {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        let (proof, fresh_target) = if let Some(memo) = &retained {
            (memo.proof.clone(), None)
        } else if let Some((target, proof)) = loaded {
            (proof, Some(target))
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            self.calls.fetch_add(1, Ordering::Relaxed);
            let tracked = self
                .inner
                .resolve_dependency_with_inputs(base, specifier, mode)?;
            tracked.inputs.validate_in(self.inner.snapshot())?;
            (
                Arc::new(Proof::Fresh(tracked.inputs)),
                Some(tracked.dependency),
            )
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
        let key = query.allocate(self.scratch);
        let target = retained
            .as_ref()
            .map(|memo| &memo.target)
            .or(fresh_target.as_ref())
            .expect("resolved target");
        let target = match target {
            ResolvedDependency::File(path) => TargetValue::File(self.scratch.path(path)),
            ResolvedDependency::External(name) => TargetValue::External(self.scratch.text(name)),
        };
        let row = self.scratch.alloc(RequestRow { key, target });
        let mut inserted = false;
        let request = self
            .requests
            .write()
            .entry(&row.key)
            .or_insert_with(|| {
                inserted = true;
                Arc::new(CurrentRequest {
                    row,
                    proof,
                    retained,
                    demanded: AtomicBool::new(false),
                    cache_hit,
                })
            })
            .clone();
        if inserted && cacheable && matches!(request.proof.as_ref(), Proof::Fresh(_)) {
            let mut packages = self.packages.write();
            let package = packages.get_mut(&package_path).expect("package");
            if package.loaded.as_ref().is_none_or(|record| {
                record
                    .find(wire::Mode(mode_id), &importer, specifier)
                    .is_none()
            }) {
                package.fresh.push(request.clone());
            }
        }
        Ok(request)
    }
}

impl DependencyResolver for BuildResolver<'_, '_> {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        Ok(self
            .resolve_request(base, specifier, mode)?
            .row
            .target
            .owned())
    }
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;
