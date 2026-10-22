//! Stage-three storage backend; intentionally not connected to BuildResolver yet.
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use cache_flatbuffers::{
    resolution::{validate_path, Key, WireResolution, SCHEMA_VERSION},
    resolution_wire as wire, MAX_RECORD_BYTES,
};
use eyre::{ensure, OptionExt, Result};
use ftree_cache::inputs::{InputKind, InputProof, PathInput};
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
    pub(crate) fn new(project: &Path, package: &Path) -> Result<Self> {
        let canonical_project = project.canonicalize()?;
        let canonical_package = package.canonicalize()?;
        let working_directory = std::env::current_dir()?;
        let canonical_working_directory = working_directory.canonicalize()?;
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

    fn validate(&self) -> Result<()> {
        ensure!(
            self.project.canonicalize()? == self.canonical_project
                && self.package.canonicalize()? == self.canonical_package
                && std::env::current_dir()? == self.working_directory
                && self.working_directory.canonicalize()? == self.canonical_working_directory,
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
        .map(|bytes| wire::DigestT { bytes: *bytes });
    record.links = links;
    Ok(record)
}

fn validate_input(roots: &Roots, input: wire::Input<'_>) -> Result<()> {
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
    PathInput::checked_from_parts(path, kind, canonical, identity, links, content)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct CheckedResolution {
    wire: WireResolution,
    roots: Roots,
}

impl CheckedResolution {
    pub(crate) fn encode(roots: Roots, key: Key, requests: &[Request<'_>]) -> Result<Self> {
        let mut entries = requests
            .iter()
            .map(|request| {
                request.inputs.validate()?;
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
        Self::check(WireResolution::encode(&data, key)?, roots)
    }

    fn check(wire: WireResolution, roots: Roots) -> Result<Self> {
        // Semantic filesystem validity precedes construction of the checked owner.
        Self::validate(&wire, &roots)?;
        Ok(Self { wire, roots })
    }

    fn validate(wire: &WireResolution, roots: &Roots) -> Result<()> {
        roots.validate()?;
        for request in wire.root().requests() {
            let importer = roots.package.join(from_portable(request.importer()));
            ensure!(importer.is_file(), "resolution importer no longer exists");
            for input in request.inputs() {
                validate_input(roots, input)?;
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
        }
        Ok(())
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        Self::validate(&self.wire, &self.roots)
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

    pub(crate) fn load(&self) -> Load {
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
            Err(error) => return miss(MissReason::ReadFailure(error.to_string())),
        };
        let mut bytes = Vec::new();
        if let Err(error) = file
            .take(MAX_RECORD_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        {
            return miss(MissReason::ReadFailure(error.to_string()));
        }
        match WireResolution::new(bytes.into(), self.key)
            .and_then(|wire| CheckedResolution::check(wire, self.roots.clone()))
        {
            Ok(record) => Load {
                record: Some(record),
                status: CacheStatus::Hit,
            },
            Err(error) => miss(MissReason::InvalidRecord(format!("{error:#}"))),
        }
    }

    pub(crate) fn save(&self, record: &CheckedResolution) -> CacheWrite {
        if self.mode != CacheMode::ReadWrite {
            return CacheWrite::NotRequested;
        }
        match self.save_inner(record) {
            Ok(()) => CacheWrite::Stored,
            Err(error) => CacheWrite::Failed(format!("{error:#}")),
        }
    }

    fn save_inner(&self, record: &CheckedResolution) -> Result<()> {
        ensure!(
            record.roots == self.roots,
            "resolution storage roots mismatch"
        );
        // A checked owner may have been made for a different policy/package.
        WireResolution::new(record.wire.bytes().into(), self.key)?;
        record.revalidate()?;
        let mut stage = tempfile::Builder::new()
            .prefix(".bundler-resolution-stage-")
            .tempfile_in(&self.roots.package)?;
        stage.write_all(record.wire.bytes())?;
        stage.flush()?;
        stage.as_file().sync_all()?;
        stage.persist(self.path())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "resolution_cache_tests.rs"]
mod tests;
