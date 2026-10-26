//! Package-local storage and borrowed request validation for production resolution.
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use cache_flatbuffers::{
    resolution::{validate_path, Key, WireResolution, SCHEMA_VERSION},
    resolution_wire as wire, MAX_RECORD_BYTES,
};
use eyre::{ensure, OptionExt, Result, WrapErr};
use ftree_cache::inputs::{InputKind, InputProof, InputSnapshot, PathInput};
use segment_cache::{CacheMode, CacheStatus, CacheWrite, MissReason};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Roots {
    project: PathBuf,
    package: PathBuf,
    canonical_project: PathBuf,
    canonical_package: PathBuf,
    native_project: PathBuf,
    native_package: PathBuf,
    working_directory: PathBuf,
    canonical_working_directory: PathBuf,
    native_working_directory: PathBuf,
}

impl Roots {
    #[cfg(test)]
    pub(crate) fn new(project: &Path, package: &Path) -> Result<Self> {
        Self::new_in(project, package, &InputSnapshot::default())
    }

    pub(crate) fn new_in(project: &Path, package: &Path, snapshot: &InputSnapshot) -> Result<Self> {
        let canonical_project = snapshot
            .capture(project)?
            .canonical()
            .map(Path::to_owned)
            .ok_or_eyre("missing resolution-cache project root")
            .wrap_err_with(|| {
                format!(
                    "resolving resolution-cache project root {}",
                    project.display()
                )
            })?;
        let canonical_package = snapshot
            .capture(package)?
            .canonical()
            .map(Path::to_owned)
            .ok_or_eyre("missing resolution-cache package root")
            .wrap_err_with(|| {
                format!(
                    "resolving resolution-cache package root {}",
                    package.display()
                )
            })?;
        let working_directory =
            std::env::current_dir().wrap_err("reading resolution-cache working directory")?;
        let canonical_working_directory = snapshot
            .capture(&working_directory)?
            .canonical()
            .map(Path::to_owned)
            .ok_or_eyre("missing resolution-cache working directory")
            .wrap_err_with(|| {
                format!(
                    "resolving resolution-cache working directory {}",
                    working_directory.display()
                )
            })?;
        Ok(Self {
            project: std::path::absolute(project)?,
            package: std::path::absolute(package)?,
            native_project: Self::native_path(&canonical_project),
            native_package: Self::native_path(&canonical_package),
            canonical_project,
            canonical_package,
            native_working_directory: Self::native_path(&canonical_working_directory),
            working_directory,
            canonical_working_directory,
        })
    }

    fn anchors(&self) -> [(wire::Anchor, &Path); 9] {
        [
            (wire::Anchor::Package, &self.package),
            (wire::Anchor::CanonicalPackage, &self.canonical_package),
            (wire::Anchor::Project, &self.project),
            (wire::Anchor::CanonicalProject, &self.canonical_project),
            (wire::Anchor::NativePackage, &self.native_package),
            (wire::Anchor::NativeProject, &self.native_project),
            (wire::Anchor::WorkingDirectory, &self.working_directory),
            (
                wire::Anchor::CanonicalWorkingDirectory,
                &self.canonical_working_directory,
            ),
            (
                wire::Anchor::NativeWorkingDirectory,
                &self.native_working_directory,
            ),
        ]
    }

    fn encode(&self, path: &Path) -> Result<Box<wire::LocatorT>> {
        ensure!(
            path.is_absolute(),
            "unportable relative filesystem observation"
        );
        for (anchor, root) in self.anchors() {
            if let Ok(relative) = path.strip_prefix(root) {
                let mut locator = wire::LocatorT::default();
                locator.anchor = anchor;
                locator.path = portable(relative)?;
                return Ok(Box::new(locator));
            }
        }
        for (anchor, root) in [
            (wire::Anchor::Project, self.project.as_path()),
            (
                wire::Anchor::CanonicalProject,
                self.canonical_project.as_path(),
            ),
            (wire::Anchor::NativeProject, self.native_project.as_path()),
        ] {
            for (parents, ancestor) in root.ancestors().enumerate().skip(1).take(64) {
                if let Ok(relative) = path.strip_prefix(ancestor) {
                    let path = portable(relative)?;
                    if path.is_empty()
                        || path == "package.json"
                        || path == "node_modules"
                        || path.starts_with("node_modules/")
                    {
                        let mut locator = wire::LocatorT::default();
                        locator.anchor = anchor;
                        locator.parents = parents as u16;
                        locator.path = path;
                        return Ok(Box::new(locator));
                    }
                }
            }
        }
        eyre::bail!(
            "resolution footprint has an uncacheable external path: {}",
            path.display()
        )
    }

    fn native_path(path: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            use std::path::Prefix;
            let mut components = path.components();
            if let Some(Component::Prefix(prefix)) = components.next() {
                let root = match prefix.kind() {
                    Prefix::VerbatimDisk(drive) => {
                        Some(PathBuf::from(format!("{}:", char::from(drive))))
                    }
                    Prefix::VerbatimUNC(server, share) => {
                        let mut root = PathBuf::from(r"\\");
                        root.push(server);
                        root.push(share);
                        Some(root)
                    }
                    _ => None,
                };
                if let Some(mut root) = root {
                    for component in components {
                        root.push(component.as_os_str());
                    }
                    return root;
                }
            }
        }
        path.to_owned()
    }

    fn decode(&self, locator: wire::Locator<'_>) -> Result<PathBuf> {
        let mut root = self
            .anchors()
            .into_iter()
            .find(|(anchor, _)| *anchor == locator.anchor())
            .ok_or_eyre("unknown resolution anchor")?
            .1;
        for _ in 0..locator.parents() {
            root = root
                .parent()
                .ok_or_eyre("resolution ancestor exceeds filesystem root")?;
        }
        Ok(root.join(from_portable(locator.path())))
    }

    fn validate_in(&self, snapshot: &InputSnapshot) -> Result<()> {
        ensure!(
            snapshot.capture(&self.project)?.canonical() == Some(self.canonical_project.as_path())
                && snapshot.capture(&self.package)?.canonical()
                    == Some(self.canonical_package.as_path())
                && std::env::current_dir()? == self.working_directory
                && snapshot.capture(&self.working_directory)?.canonical()
                    == Some(self.canonical_working_directory.as_path()),
            "resolution path anchors changed"
        );
        Ok(())
    }
}

fn portable(path: &Path) -> Result<String> {
    let parts = path
        .components()
        .map(|part| match part {
            Component::Normal(part) => part.to_str().ok_or_eyre("non-UTF8 resolution path"),
            _ => Err(eyre::eyre!("unsafe resolution path component")),
        })
        .collect::<Result<Vec<_>>>()?;
    let path = parts.join("/");
    validate_path(&path, true)?;
    Ok(path)
}

fn from_portable(path: &str) -> PathBuf {
    path.split('/').filter(|part| !part.is_empty()).collect()
}

fn normalize(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => ensure!(result.pop(), "link target escapes filesystem root"),
            Component::CurDir => {}
            part => result.push(part.as_os_str()),
        }
    }
    Ok(result)
}

pub(crate) enum Target<'a> {
    File(&'a Path),
    External(&'a str),
}

/// Cold-path inputs only; persisted reads retain borrowed tables instead.
pub(crate) struct Request<'a> {
    pub(crate) mode: wire::Mode,
    pub(crate) importer: &'a str,
    pub(crate) specifier: &'a str,
    pub(crate) target: Target<'a>,
    pub(crate) inputs: &'a InputProof,
}

fn kind(kind: InputKind) -> wire::Kind {
    match kind {
        InputKind::Missing => wire::Kind::Missing,
        InputKind::File => wire::Kind::File,
        InputKind::Directory => wire::Kind::Directory,
        InputKind::Other => wire::Kind::Other,
    }
}

fn encode_input(roots: &Roots, input: &PathInput) -> Result<wire::InputT> {
    let links = input
        .links()
        .iter()
        .map(|(path, target)| {
            let relative = if target.is_relative() {
                let spelling = target.to_str().ok_or_eyre("non-UTF8 link target")?;
                #[cfg(windows)]
                let spelling = spelling.replace('\\', "/");
                #[cfg(not(windows))]
                let spelling = spelling.to_owned();
                Some(spelling)
            } else {
                None
            };
            let resolved = if target.is_relative() {
                normalize(&path.parent().ok_or_eyre("link has no parent")?.join(target))?
            } else {
                target.clone()
            };
            let mut link = wire::LinkT::default();
            link.path = roots.encode(path)?;
            link.target = roots.encode(&resolved)?;
            link.relative = relative;
            Ok(link)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut record = wire::InputT::default();
    record.path = roots.encode(input.path())?;
    record.kind = kind(input.kind());
    record.canonical = input
        .canonical()
        .map(|path| roots.encode(path))
        .transpose()?;
    record.identity = input
        .identity()
        .map(|(volume, file)| wire::IdentityT { volume, file });
    record.content = input
        .content_seal()
        .map(|bytes| wire::Hash128T { bytes: *bytes });
    record.links = links;
    Ok(record)
}

fn validate_input(roots: &Roots, input: wire::Input<'_>, snapshot: &InputSnapshot) -> Result<()> {
    let path = roots.decode(input.path())?;
    let canonical = input
        .canonical()
        .map(|path| roots.decode(path))
        .transpose()?;
    let identity = input.identity().map(|id| (id.volume(), id.file()));
    let content = input
        .content()
        .map(|digest| std::array::from_fn(|index| digest.bytes().get(index)));
    let kind = match input.kind() {
        wire::Kind::Missing => InputKind::Missing,
        wire::Kind::File => InputKind::File,
        wire::Kind::Directory => InputKind::Directory,
        wire::Kind::Other => InputKind::Other,
        _ => eyre::bail!("unknown resolution input kind"),
    };
    let mut links = Vec::with_capacity(input.links().len());
    for link in input.links() {
        let path = roots.decode(link.path())?;
        let resolved = roots.decode(link.target())?;
        let target = match link.relative() {
            Some(relative) => {
                let relative = from_portable(relative);
                ensure!(
                    normalize(
                        &path
                            .parent()
                            .ok_or_eyre("link has no parent")?
                            .join(&relative)
                    )? == resolved,
                    "resolution link target mismatch"
                );
                relative
            }
            None => resolved,
        };
        if let Some((previous, _)) = links.last() {
            ensure!(
                path != *previous && path.starts_with(previous),
                "unordered resolution link prefixes"
            );
        }
        links.push((path, target));
    }
    // Only one proof at a time is reconstructed, checked, then discarded.
    // The checked cache itself retains no unpacked observations or tree.
    PathInput::checked_from_parts_in(snapshot, path, kind, canonical, identity, links, content)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct CheckedResolution {
    wire: WireResolution,
    roots: Roots,
}

impl CheckedResolution {
    #[cfg(test)]
    pub(crate) fn encode(roots: Roots, key: Key, requests: &[Request<'_>]) -> Result<Self> {
        Self::encode_in(roots, key, requests, &InputSnapshot::default())
    }

    pub(crate) fn encode_in(
        roots: Roots,
        key: Key,
        requests: &[Request<'_>],
        snapshot: &InputSnapshot,
    ) -> Result<Self> {
        let mut entries = requests
            .iter()
            .map(|request| {
                request.inputs.validate_in(snapshot)?;
                let mut target = wire::TargetT::default();
                match request.target {
                    Target::File(path) => target.file = Some(roots.encode(path)?),
                    Target::External(external) => {
                        target.kind = wire::TargetKind::External;
                        target.external = Some(external.to_owned());
                    }
                }
                let mut entry = wire::RequestT::default();
                entry.mode = request.mode;
                entry.importer = request.importer.to_owned();
                entry.specifier = request.specifier.to_owned();
                entry.target = Box::new(target);
                entry.inputs = request
                    .inputs
                    .inputs()
                    .iter()
                    .map(|input| encode_input(&roots, input))
                    .collect::<Result<Vec<_>>>()?;
                Ok(entry)
            })
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|a, b| {
            (a.mode.0, &a.importer, &a.specifier).cmp(&(b.mode.0, &b.importer, &b.specifier))
        });
        let mut data = wire::ResolutionT::default();
        data.schema = SCHEMA_VERSION;
        data.policy = wire::DigestT { bytes: key.policy };
        data.instance = wire::DigestT {
            bytes: key.instance,
        };
        data.dependencies = wire::DigestT {
            bytes: key.dependencies,
        };
        data.requests = entries;
        Self::check_in(WireResolution::encode(&data, key)?, roots, snapshot)
    }

    fn check_in(wire: WireResolution, roots: Roots, snapshot: &InputSnapshot) -> Result<Self> {
        // Semantic filesystem validity precedes construction of the checked owner.
        Self::validate_in(&wire, &roots, snapshot)?;
        Ok(Self { wire, roots })
    }

    fn validate_in(wire: &WireResolution, roots: &Roots, snapshot: &InputSnapshot) -> Result<()> {
        roots.validate_in(snapshot)?;
        for request in wire.root().requests() {
            Self::validate_request(roots, request, snapshot)?;
        }
        Ok(())
    }

    fn validate_request(
        roots: &Roots,
        request: wire::Request<'_>,
        snapshot: &InputSnapshot,
    ) -> Result<()> {
        let importer = roots.package.join(from_portable(request.importer()));
        ensure!(
            snapshot.capture(&importer)?.kind() == InputKind::File,
            "resolution importer no longer exists"
        );
        for input in request.inputs() {
            validate_input(roots, input, snapshot)?;
        }
        if let Some(target) = request.target().file() {
            let target = roots.decode(target)?;
            ensure!(
                request.inputs().iter().any(|input| {
                    input.kind() == wire::Kind::File
                        && input
                            .canonical()
                            .is_some_and(|p| roots.decode(p).is_ok_and(|p| p == target))
                }),
                "resolution file target lacks a positive canonical proof"
            );
        }
        Ok(())
    }

    pub(crate) fn revalidate_request_in(
        &self,
        mode: wire::Mode,
        importer: &str,
        specifier: &str,
        snapshot: &InputSnapshot,
    ) -> Result<()> {
        self.roots.validate_in(snapshot)?;
        Self::validate_request(
            &self.roots,
            self.find(mode, importer, specifier)
                .ok_or_eyre("missing resolution request")?,
            snapshot,
        )
    }

    /// Project only positive lexical routes from the borrowed request. No proof
    /// tree is unpacked, and validation remains the original owner's authority.
    pub(crate) fn file_routes(
        &self,
        mode: wire::Mode,
        importer: &str,
        specifier: &str,
        target: &Path,
    ) -> Result<Vec<PathBuf>> {
        let request = self
            .find(mode, importer, specifier)
            .ok_or_eyre("missing request")?;
        let mut routes = Vec::new();
        for input in request.inputs() {
            if !matches!(input.kind(), wire::Kind::File | wire::Kind::Directory) {
                continue;
            }
            let Some(canonical) = input.canonical() else {
                continue;
            };
            let canonical = self.roots.decode(canonical)?;
            if let Ok(relative) = target.strip_prefix(&canonical) {
                let path = self.roots.decode(input.path())?;
                routes.push(if relative.as_os_str().is_empty() {
                    path
                } else {
                    path.join(relative)
                });
            }
        }
        Ok(routes)
    }

    pub(crate) fn aggregate_in(
        roots: Roots,
        key: Key,
        records: &[Self],
        snapshot: &InputSnapshot,
    ) -> Result<Self> {
        let mut requests = std::collections::BTreeMap::new();
        for record in records {
            ensure!(roots == record.roots, "resolution merge roots mismatch");
            WireResolution::new(record.wire.bytes().into(), key)?;
            for request in record.requests() {
                requests.insert(
                    (request.mode().0, request.importer(), request.specifier()),
                    request,
                );
            }
        }
        Self::check_in(
            WireResolution::aggregate(key, &requests.into_values().collect::<Vec<_>>())?,
            roots,
            snapshot,
        )
    }

    #[cfg(test)]
    pub(crate) fn revalidate(&self) -> Result<()> {
        self.revalidate_in(&InputSnapshot::default())
    }

    pub(crate) fn revalidate_in(&self, snapshot: &InputSnapshot) -> Result<()> {
        Self::validate_in(&self.wire, &self.roots, snapshot)
    }

    pub(crate) fn requests(&self) -> impl Iterator<Item = wire::Request<'_>> {
        self.wire.root().requests().iter()
    }

    pub(crate) fn find(
        &self,
        mode: wire::Mode,
        importer: &str,
        specifier: &str,
    ) -> Option<wire::Request<'_>> {
        let requests = self.wire.root().requests();
        let key = (mode.0, importer, specifier);
        let (mut low, mut high) = (0, requests.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let request = requests.get(middle);
            match (request.mode().0, request.importer(), request.specifier()).cmp(&key) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(request),
            }
        }
        None
    }

    pub(crate) fn file_target(
        &self,
        mode: wire::Mode,
        importer: &str,
        specifier: &str,
    ) -> Result<Option<PathBuf>> {
        self.find(mode, importer, specifier)
            .and_then(|request| request.target().file())
            .map(|path| self.roots.decode(path))
            .transpose()
    }
}

pub(crate) struct Load {
    pub(crate) record: Option<CheckedResolution>,
    pub(crate) status: CacheStatus,
}

pub(crate) struct Store {
    roots: Roots,
    key: Key,
    mode: CacheMode,
}

impl Store {
    pub(crate) fn new(roots: Roots, key: Key, mode: CacheMode) -> Self {
        Self { roots, key, mode }
    }

    fn path(&self) -> PathBuf {
        let policy = self
            .key
            .policy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.roots
            .package
            .join(format!(".bundler-resolution-v{SCHEMA_VERSION}-{policy}.fb"))
    }

    #[cfg(test)]
    pub(crate) fn load(&self) -> Load {
        self.load_inner(false, &InputSnapshot::default())
    }

    /// A stale request must not evict independent, still-proven requests.
    #[cfg(test)]
    pub(crate) fn load_scoped(&self) -> Load {
        self.load_scoped_in(&InputSnapshot::default())
    }

    pub(crate) fn load_scoped_in(&self, snapshot: &InputSnapshot) -> Load {
        self.load_inner(true, snapshot)
    }

    fn load_inner(&self, scoped: bool, snapshot: &InputSnapshot) -> Load {
        let miss = |reason| Load {
            record: None,
            status: CacheStatus::Miss(reason),
        };
        if self.mode == CacheMode::Disabled {
            return miss(MissReason::Disabled);
        }
        let file = match File::open(self.path()) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return miss(MissReason::Missing)
            }
            Err(error) => {
                return miss(MissReason::ReadFailure(format!(
                    "opening resolution cache {}: {error}",
                    self.path().display()
                )))
            }
        };
        let mut bytes = Vec::new();
        if let Err(error) = file
            .take(MAX_RECORD_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        {
            return miss(MissReason::ReadFailure(format!(
                "reading resolution cache {}: {error}",
                self.path().display()
            )));
        }
        match WireResolution::new(bytes.into(), self.key).and_then(|wire| {
            if !scoped {
                return CheckedResolution::check_in(wire, self.roots.clone(), snapshot);
            }
            self.roots.validate_in(snapshot)?;
            let requests: Vec<_> = wire
                .root()
                .requests()
                .iter()
                .filter(|r| CheckedResolution::validate_request(&self.roots, *r, snapshot).is_ok())
                .collect();
            if requests.len() == wire.root().requests().len() {
                return Ok(CheckedResolution {
                    wire,
                    roots: self.roots.clone(),
                });
            }
            let wire = WireResolution::aggregate(self.key, &requests)?;
            CheckedResolution::check_in(wire, self.roots.clone(), snapshot)
        }) {
            Ok(record) => Load {
                record: Some(record),
                status: CacheStatus::Hit,
            },
            Err(error) => miss(MissReason::InvalidRecord(format!("{error:#}"))),
        }
    }

    #[cfg(test)]
    pub(crate) fn save(&self, record: &CheckedResolution) -> CacheWrite {
        self.save_in(record, &InputSnapshot::default())
    }

    pub(crate) fn save_in(
        &self,
        record: &CheckedResolution,
        snapshot: &InputSnapshot,
    ) -> CacheWrite {
        if self.mode != CacheMode::ReadWrite {
            return CacheWrite::NotRequested;
        }
        match self.save_inner(record, snapshot) {
            Ok(()) => CacheWrite::Stored,
            Err(error) => CacheWrite::Failed(format!(
                "saving resolution cache {}: {error:#}",
                self.path().display()
            )),
        }
    }

    fn save_inner(&self, record: &CheckedResolution, snapshot: &InputSnapshot) -> Result<()> {
        ensure!(
            record.roots == self.roots,
            "resolution storage roots mismatch"
        );
        // A checked owner may have been made for a different policy/package.
        WireResolution::new(record.wire.bytes().into(), self.key)?;
        record.revalidate_in(snapshot)?;
        let mut stage = tempfile::Builder::new()
            .prefix(".bundler-resolution-stage-")
            .tempfile_in(&self.roots.package)
            .wrap_err("creating resolution-cache staging file")?;
        stage
            .write_all(record.wire.bytes())
            .wrap_err("writing resolution-cache staging file")?;
        stage
            .flush()
            .wrap_err("flushing resolution-cache staging file")?;
        stage
            .as_file()
            .sync_all()
            .wrap_err("syncing resolution-cache staging file")?;
        stage
            .persist(self.path())
            .wrap_err("replacing resolution cache")?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "resolution_cache_tests.rs"]
mod tests;
