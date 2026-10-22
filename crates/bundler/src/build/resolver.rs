use std::cell::{Cell, RefCell};

use cache_flatbuffers::{resolution::Key, resolution_wire as wire};
use ftree_cache::inputs::{InputKind, InputProof, PathInput};

use super::*;
use crate::{
    lockfile::ProjectLockfile,
    resolution_cache::{CheckedResolution, Request, Roots, Store, Target},
};

type RequestKey = (u8, String, String);
type MemoKey = ([u8; 32], [u8; 32], [u8; 32], PathBuf, u8, String);

#[derive(Clone)]
enum Proof {
    Fresh(InputProof),
    Disk(Rc<CheckedResolution>, RequestKey),
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

    fn validate(&self) -> eyre::Result<()> {
        match self {
            Self::Fresh(inputs) => inputs.validate(),
            Self::Disk(record, (mode, importer, specifier)) => {
                record.revalidate_request(wire::Mode(*mode), importer, specifier)
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
        let target = self.resolver.resolve_dependency(base, specifier, mode)?;
        if let ResolvedDependency::File(path) = &target {
            let mode_id = if mode == ResolutionMode::Import { 0 } else { 1 };
            let package = self.resolver.package(base)?;
            let key = self.resolver.packages.borrow()[&package].key;
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
                .borrow()
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
                        if let Ok(canonical) = alias.canonicalize() {
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
    proof: Rc<Proof>,
}

#[derive(Clone, Default)]
pub(super) struct Memory(Rc<RefCell<BTreeMap<MemoKey, Memo>>>);

impl Memory {
    pub(super) fn retain_used(&self, script: &BuildResolver, sass: &BuildResolver) {
        let script = script.used.borrow();
        let sass = sass.used.borrow();
        self.0
            .borrow_mut()
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
    pub(super) fn read(root: &Path, warnings: &mut Vec<String>) -> eyre::Result<Rc<Self>> {
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
            ProjectLockfile::read(root, None)
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
        let snapshot = Rc::new(Self {
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
    loaded: Option<Rc<CheckedResolution>>,
    fresh: BTreeMap<RequestKey, Memo>,
}

/// The source-edge map is derived state, never a resolution authority. Each
/// build consults actual requests and retains their original proof until return.
pub(super) struct BuildResolver {
    root: PathBuf,
    inner: StructuralResolver,
    inner_options: NodeModulesResolverOptions,
    policy: [u8; 32],
    mode: CacheMode,
    lockfile: Rc<Lockfile>,
    memory: Memory,
    used: RefCell<BTreeMap<MemoKey, Memo>>,
    packages: RefCell<BTreeMap<PathBuf, Package>>,
    bases: RefCell<BTreeMap<PathBuf, PathBuf>>,
    package_inputs: RefCell<Vec<PathInput>>,
    warnings: RefCell<Vec<String>>,
    calls: Cell<usize>,
    hits: Cell<usize>,
    misses: Cell<usize>,
}

impl BuildResolver {
    pub(super) fn admitting(self, inventory: &Inventory) -> Admitting<'_> {
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
        lockfile: Rc<Lockfile>,
        memory: Memory,
    ) -> Self {
        Self {
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
            used: RefCell::new(BTreeMap::new()),
            packages: RefCell::new(BTreeMap::new()),
            bases: RefCell::new(BTreeMap::new()),
            package_inputs: RefCell::new(Vec::new()),
            warnings: RefCell::new(Vec::new()),
            calls: Cell::new(0),
            hits: Cell::new(0),
            misses: Cell::new(0),
        }
    }

    fn package(&self, base: &Path) -> eyre::Result<PathBuf> {
        let directory = base
            .parent()
            .ok_or_else(|| eyre::eyre!("resolution base has no parent"))?;
        if let Some(package) = self.bases.borrow().get(directory) {
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
            if self.packages.borrow().contains_key(directory) {
                package = directory.into();
                break;
            }
            let path = directory.join("package.json");
            let mut input = PathInput::capture(&path)?;
            if input.kind() == InputKind::File {
                let bytes = fs::read(&path)?;
                input.seal_content(&bytes);
                input.validate()?;
                package = directory.into();
                manifest_bytes = Some(bytes);
                self.package_inputs.borrow_mut().push(input);
                break;
            }
            self.package_inputs.borrow_mut().push(input);
            if directory == self.root {
                break;
            }
        }
        if !self.packages.borrow().contains_key(&package) {
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
            let roots = Roots::new(&self.root, &package)?;
            let store = Store::new(roots.clone(), key, self.mode);
            let load = store.load_scoped();
            if let CacheStatus::Miss(
                reason @ (segment_cache::MissReason::InvalidRecord(_)
                | segment_cache::MissReason::ReadFailure(_)),
            ) = &load.status
            {
                self.warnings.borrow_mut().push(format!(
                    "{}: resolution cache miss: {reason:?}",
                    package.display()
                ));
            }
            self.packages.borrow_mut().insert(
                package.clone(),
                Package {
                    roots,
                    key,
                    store,
                    loaded: load.record.map(Rc::new),
                    fresh: BTreeMap::new(),
                },
            );
        }
        self.bases
            .borrow_mut()
            .insert(directory.into(), package.clone());
        Ok(package)
    }

    pub(super) fn recheck(&self) -> eyre::Result<()> {
        self.lockfile.recheck()?;
        for input in self.package_inputs.borrow().iter() {
            input.validate()?;
        }
        for memo in self.used.borrow().values() {
            memo.proof.validate()?;
        }
        Ok(())
    }

    /// Called only after all build inputs have passed final validation.
    pub(super) fn finish(&self, counters: &mut Counters, warnings: &mut Vec<String>) {
        counters.resolutions += self.calls.get();
        counters.resolution_cache_hits += self.hits.get();
        counters.resolution_cache_misses += self.misses.get();
        warnings.extend(self.warnings.borrow().iter().cloned());
        if self.mode != CacheMode::ReadWrite {
            return;
        }
        for (path, package) in self.packages.borrow().iter() {
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
                let encoded = CheckedResolution::encode(
                    package.roots.clone(),
                    package.key,
                    &[Request {
                        mode: wire::Mode(*mode),
                        importer,
                        specifier,
                        target,
                        inputs,
                    }],
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
            match CheckedResolution::aggregate(package.roots.clone(), package.key, &records) {
                Ok(record) => {
                    if let CacheWrite::Failed(error) = package.store.save(&record) {
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

impl DependencyResolver for BuildResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
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
        let key = self.packages.borrow()[&package_path].key;
        let memo_key = (
            key.policy,
            key.instance,
            key.dependencies,
            base.into(),
            mode_id,
            specifier.into(),
        );
        if let Some(memo) = self.used.borrow().get(&memo_key) {
            return Ok(memo.target.clone());
        }
        let memory = self.memory.0.borrow().get(&memo_key).cloned();
        let mut memo = memory.filter(|memo| memo.proof.validate().is_ok());
        if memo.is_none() && cacheable {
            let packages = self.packages.borrow();
            if let Some(record) = &packages[&package_path].loaded {
                if let Some(request) = record.find(wire::Mode(mode_id), &request_key.1, specifier) {
                    record.revalidate_request(wire::Mode(mode_id), &request_key.1, specifier)?;
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
                        proof: Rc::new(Proof::Disk(record.clone(), request_key.clone())),
                    });
                }
            }
        }
        let memo = match memo {
            Some(memo) => {
                self.hits.set(self.hits.get() + 1);
                memo
            }
            None => {
                self.misses.set(self.misses.get() + 1);
                self.calls.set(self.calls.get() + 1);
                let tracked = self
                    .inner
                    .resolve_dependency_with_inputs(base, specifier, mode)?;
                tracked.inputs.validate()?;
                Memo {
                    target: tracked.dependency,
                    proof: Rc::new(Proof::Fresh(tracked.inputs)),
                }
            }
        };
        if cacheable && matches!(memo.proof.as_ref(), Proof::Fresh(_)) {
            let mut packages = self.packages.borrow_mut();
            let package = packages.get_mut(&package_path).expect("package");
            if package.loaded.as_ref().is_none_or(|record| {
                record
                    .find(wire::Mode(mode_id), &request_key.1, specifier)
                    .is_none()
            }) {
                package.fresh.insert(request_key, memo.clone());
            }
        }
        self.used
            .borrow_mut()
            .insert(memo_key.clone(), memo.clone());
        self.memory.0.borrow_mut().insert(memo_key, memo.clone());
        Ok(memo.target)
    }
}
