//! Immutable generations with one atomic ownership/current-generation pointer.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use eyre::{ensure, WrapErr};
use serde::{Deserialize, Serialize};

use crate::{
    build::{hash, BuildOptions, BuildResult, BuildSession},
    link::Artifacts,
};

const STATE: &str = ".bundler-current.json";
const LOCK: &str = ".bundler-lock";
const OWNER: &str = "good-fences-bundler-publication-v1";
const MAX_METADATA: u64 = 4 * 1024 * 1024;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> eyre::Result<()> {
        ensure!(
            !self.0.load(Ordering::Acquire),
            "build cancelled before publication commit"
        );
        Ok(())
    }
}

#[derive(Debug)]
pub struct Published {
    pub directory: PathBuf,
    pub entries: BTreeMap<String, PathBuf>,
    pub changed: bool,
    /// Successfully staged artifact files; excludes the atomic pointer metadata.
    pub files_written: usize,
    /// Commit succeeded; an OS lock-release failure is reported, not rolled back.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct State {
    owner: String,
    current: String,
    generations: BTreeMap<String, BTreeMap<PathBuf, [u8; 32]>>,
}

#[derive(Clone, Copy)]
enum Phase {
    Staged,
    Installed,
    BeforeCommit,
}

/// Publication is nonblocking single-writer: an overlapping publisher receives
/// an error. Sequential explicit snapshots follow commit order.
pub fn publish(
    output: &Path,
    artifacts: &Artifacts,
    cancel: &Cancellation,
) -> eyre::Result<Published> {
    publish_inner(output, artifacts, cancel, |_| Ok(()))
}

impl BuildSession {
    /// Builds in memory first; failures/cancellation never change the output
    /// pointer. Cancellation after the atomic commit cannot roll back a success.
    pub fn build_and_publish(
        &mut self,
        options: &BuildOptions,
        output: &Path,
        cancel: &Cancellation,
    ) -> eyre::Result<(BuildResult, Published)> {
        cancel.check()?;
        ensure!(
            !output
                .components()
                .any(|c| matches!(c, Component::ParentDir)),
            "output path must not contain parent traversal"
        );
        let absolute = if output.is_absolute() {
            output.to_owned()
        } else {
            std::env::current_dir()?.join(output)
        };
        if let Ok(relative) = absolute.strip_prefix(self.root()) {
            ensure!(
                relative.components().any(|c| c
                    .as_os_str()
                    .to_string_lossy()
                    .starts_with(".bundler-")
                    || c.as_os_str() == "target"),
                "in-project output must be inside a reserved .bundler-* or target directory"
            );
        }
        let (output, lock) = lock_output(&absolute)?;
        let result = (|| {
            let build = self.build(options)?;
            cancel.check()?;
            let published = publish_locked(&output, &build.product.artifacts, cancel, |_| Ok(()))
                .wrap_err_with(|| {
                format!("publication failed; cache warnings: {:?}", build.warnings)
            })?;
            Ok((build, published))
        })();
        finish_lock(lock, result, |(_, published)| &mut published.warnings)
    }
}

/// Exclude the whole output tree, sidecars and all reserved staging/metadata.
/// `output` and `path` must use the same absolute/relative coordinate system.
pub fn is_generated_artifact(path: &Path, output: &Path) -> bool {
    path.starts_with(output)
        || segment_cache::is_cache_artifact(path)
        || path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with(".bundler-"))
}

fn publish_inner(
    output: &Path,
    artifacts: &Artifacts,
    cancel: &Cancellation,
    hook: impl FnMut(Phase) -> eyre::Result<()>,
) -> eyre::Result<Published> {
    cancel.check()?;
    validate_artifacts(artifacts)?;
    let (output, lock) = lock_output(output)?;
    let result = publish_locked(&output, artifacts, cancel, hook);
    finish_lock(lock, result, |published| &mut published.warnings)
}

fn validate_artifacts(artifacts: &Artifacts) -> eyre::Result<()> {
    ensure!(
        !artifacts.files.is_empty(),
        "cannot publish an empty artifact map"
    );
    let mut paths = BTreeSet::new();
    for (path, _) in artifacts.file_bytes() {
        relative(path)?;
        ensure!(
            paths.insert(path.to_string_lossy().to_lowercase()),
            "duplicate owned artifact path: {}",
            path.display()
        );
    }
    for path in &paths {
        ensure!(
            !Path::new(path)
                .ancestors()
                .skip(1)
                .any(|parent| parent.to_str().is_some_and(|parent| paths.contains(parent))),
            "owned artifact file/directory collision: {path}"
        );
    }
    for path in artifacts
        .modules
        .values()
        .chain(artifacts.entries.values())
        .chain(artifacts.members.values())
        .chain(artifacts.chunk_artifacts.values().flatten())
        .chain(artifacts.cjs_proxies.values().map(|(path, _)| path))
        .chain(artifacts.cjs_namespaces.values())
        .chain(artifacts.async_facades.values())
    {
        ensure!(
            artifacts.files.contains_key(path)
                && path
                    .extension()
                    .is_some_and(|e| e == "mjs" || e == "cjs" || e == "js"),
            "module route is not an owned executable JavaScript file: {}",
            path.display()
        );
        if path.extension().is_some_and(|e| e == "js") {
            let package = path.parent().unwrap().join("package.json");
            let metadata: serde_json::Value =
                serde_json::from_str(artifacts.files.get(&package).ok_or_else(|| {
                    eyre::eyre!(
                        "JavaScript module route requires owned module type metadata: {}",
                        path.display()
                    )
                })?)?;
            ensure!(
                metadata.as_object().is_some_and(|object| object.len() == 1)
                    && matches!(
                        metadata.get("type").and_then(|value| value.as_str()),
                        Some("module" | "commonjs")
                    ),
                "invalid generated module type metadata: {}",
                package.display()
            );
        }
    }
    Ok(())
}

struct OutputLock(Option<File>);

impl OutputLock {
    fn release(mut self) -> eyre::Result<()> {
        self.0
            .take()
            .expect("held output lock")
            .unlock()
            .wrap_err("output lock release failed")
    }
}

impl Drop for OutputLock {
    fn drop(&mut self) {
        if let Some(file) = &self.0 {
            if let Err(error) = file.unlock() {
                eprintln!("bundler: output lock release during unwind failed: {error}");
            }
        }
    }
}

fn finish_lock<T>(
    lock: OutputLock,
    result: eyre::Result<T>,
    warnings: impl FnOnce(&mut T) -> &mut Vec<String>,
) -> eyre::Result<T> {
    let release = lock.release();
    match result {
        Ok(mut value) => {
            if let Err(error) = release {
                warnings(&mut value).push(format!("{error:#}"));
            }
            Ok(value)
        }
        Err(error) => Err(match release {
            Ok(()) => error,
            Err(release) => error.wrap_err(format!("output lock cleanup also failed: {release:#}")),
        }),
    }
}

fn lock_output(output: &Path) -> eyre::Result<(PathBuf, OutputLock)> {
    fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    no_symlink(&output.join(LOCK))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(output.join(LOCK))?;
    lock.try_lock()
        .wrap_err("another publisher owns this output directory")?;
    Ok((output, OutputLock(Some(lock))))
}

fn publish_locked(
    output: &Path,
    artifacts: &Artifacts,
    cancel: &Cancellation,
    mut hook: impl FnMut(Phase) -> eyre::Result<()>,
) -> eyre::Result<Published> {
    cancel.check()?;
    validate_artifacts(artifacts)?;
    let previous = read_state(output)?;
    let owned: BTreeMap<_, _> = artifacts
        .file_bytes()
        .map(|(p, bytes)| (p.clone(), hash(bytes)))
        .collect();
    let generation = format!(
        ".bundler-generation-{}",
        hex(&hash(&serde_json::to_vec(&owned)?))
    );
    let destination = output.join(&generation);
    let mut files_written = 0;
    let mut state = previous.clone().unwrap_or(State {
        owner: OWNER.into(),
        current: generation.clone(),
        generations: BTreeMap::new(),
    });
    if let Some(existing) = state.generations.get(&generation) {
        ensure!(*existing == owned, "generation metadata mismatch");
        verify(&destination, &owned)?;
    } else {
        ensure!(
            !destination.try_exists()?,
            "refusing to adopt unowned generation {}",
            destination.display()
        );
        let stage = create_stage(output)?;
        let staged = (|| {
            for (path, bytes) in artifacts.file_bytes() {
                cancel.check()?;
                let file = stage.join(path);
                fs::create_dir_all(file.parent().expect("artifact parent"))?;
                let mut file = OpenOptions::new().write(true).create_new(true).open(file)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                files_written += 1;
            }
            hook(Phase::Staged)?;
            cancel.check()?;
            fs::rename(&stage, &destination)?;
            Ok::<_, eyre::Report>(())
        })();
        if let Err(error) = staged {
            return Err(clean_failure(error, &stage, &owned));
        }
        let ready = (|| {
            hook(Phase::Installed)?;
            cancel.check()?;
            Ok::<_, eyre::Report>(())
        })();
        if let Err(error) = ready {
            return Err(clean_failure(error, &destination, &owned));
        }
        state.generations.insert(generation.clone(), owned.clone());
    }
    let before_commit = (|| {
        hook(Phase::BeforeCommit)?;
        cancel.check()
    })();
    if let Err(error) = before_commit {
        if previous
            .as_ref()
            .is_none_or(|s| !s.generations.contains_key(&generation))
        {
            return Err(clean_failure(error, &destination, &owned));
        }
        return Err(error);
    }
    state.current = generation.clone();
    let changed = previous.as_ref() != Some(&state);
    if changed {
        if let Err(error) = write_state(output, &state) {
            if previous
                .as_ref()
                .is_none_or(|s| !s.generations.contains_key(&generation))
            {
                return Err(clean_failure(error, &destination, &owned));
            }
            return Err(error);
        }
    }
    Ok(Published {
        directory: destination.clone(),
        entries: artifacts
            .entries
            .iter()
            .map(|(n, p)| (n.clone(), destination.join(p)))
            .collect(),
        changed,
        files_written,
        warnings: vec![],
    })
}

/// Read the atomic pointer for a fresh runtime instance. Existing instances keep
/// their immutable generation URLs; publishing never changes those bytes.
pub fn current_directory(output: &Path) -> eyre::Result<PathBuf> {
    let state = read_state(output)?.ok_or_else(|| eyre::eyre!("no published generation"))?;
    let directory = output.join(state.current);
    no_symlink(&directory)?;
    ensure!(directory.is_dir(), "published generation is missing");
    Ok(directory)
}

#[derive(Debug, Default)]
pub struct Cleanup {
    pub removed: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// Caller must first quiesce runtimes using prior generations. Automatic pruning
/// would break deferred imports in still-running old runtimes.
pub fn prune_stale(output: &Path) -> eyre::Result<Cleanup> {
    let output = output.canonicalize()?;
    let (output, lock) = lock_output(&output)?;
    let result = prune_locked(&output);
    finish_lock(lock, result, |cleanup| &mut cleanup.warnings)
}

fn prune_locked(output: &Path) -> eyre::Result<Cleanup> {
    let Some(mut state) = read_state(output)? else {
        return Ok(Cleanup::default());
    };
    let mut result = Cleanup::default();
    for (name, owned) in state.generations.clone() {
        if name == state.current {
            continue;
        }
        let directory = output.join(&name);
        match cleanup(&directory, &owned) {
            Ok(()) => {
                state.generations.remove(&name);
                result.removed.push(directory);
            }
            Err(error) => result.warnings.push(format!("retaining {name}: {error:#}")),
        }
    }
    if !result.removed.is_empty() {
        write_state(output, &state)?;
    }
    Ok(result)
}

fn relative(path: &Path) -> eyre::Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "unsafe owned artifact path: {}",
        path.display()
    );
    ensure!(
        path.to_str()
            .is_some_and(|p| !p.contains('\\') && !p.contains(':') && !p.contains('\0')),
        "nonportable artifact path"
    );
    Ok(())
}

fn no_symlink(path: &Path) -> eyre::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "refusing symlink: {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn owned_path(root: &Path, path: &Path) -> eyre::Result<PathBuf> {
    relative(path)?;
    no_symlink(root)?;
    let mut file = root.to_owned();
    for component in path.components() {
        file.push(component);
        no_symlink(&file)?;
    }
    Ok(file)
}

fn verify(root: &Path, owned: &BTreeMap<PathBuf, [u8; 32]>) -> eyre::Result<()> {
    for (path, digest) in owned {
        let file = owned_path(root, path)?;
        ensure!(
            hash(&fs::read(&file)?) == *digest,
            "owned artifact modified: {}",
            file.display()
        );
    }
    Ok(())
}

fn read_state(output: &Path) -> eyre::Result<Option<State>> {
    let path = output.join(STATE);
    no_symlink(&path)?;
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_METADATA + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_METADATA,
        "ownership metadata exceeds limit"
    );
    let state: State = serde_json::from_slice(&bytes)?;
    ensure!(
        state.owner == OWNER && state.generations.contains_key(&state.current),
        "invalid publication owner/current"
    );
    for (name, owned) in &state.generations {
        ensure!(
            *name
                == format!(
                    ".bundler-generation-{}",
                    hex(&hash(&serde_json::to_vec(owned)?))
                ),
            "invalid generation ownership identity"
        );
        for path in owned.keys() {
            relative(path)?;
        }
    }
    Ok(Some(state))
}

fn write_state(output: &Path, state: &State) -> eyre::Result<()> {
    let bytes = serde_json::to_vec(state)?;
    ensure!(
        bytes.len() as u64 <= MAX_METADATA,
        "ownership metadata exceeds limit"
    );
    let mut stage = tempfile::Builder::new()
        .prefix(".bundler-pointer-stage-")
        .tempfile_in(output)?;
    stage.write_all(&bytes)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    stage
        .persist(output.join(STATE))
        .map_err(|e| eyre::eyre!("atomic output pointer replacement: {e}"))?;
    Ok(())
}

fn create_stage(output: &Path) -> eyre::Result<PathBuf> {
    loop {
        let stage = output.join(format!(
            ".bundler-stage-{}-{}",
            std::process::id(),
            NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&stage) {
            Ok(()) => return Ok(stage),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
}

/// Remove only named, unmodified owned files, then empty known parents.
/// Unknown/user-modified files and nonempty directories are never removed.
fn cleanup(root: &Path, owned: &BTreeMap<PathBuf, [u8; 32]>) -> eyre::Result<()> {
    let mut directories = BTreeSet::from([root.to_owned()]);
    for (path, digest) in owned {
        let file = owned_path(root, path)?;
        match fs::read(&file) {
            Ok(bytes) => {
                ensure!(
                    hash(&bytes) == *digest,
                    "refusing removal of modified artifact {}",
                    file.display()
                );
                fs::remove_file(&file)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        for parent in file.parent().into_iter().flat_map(Path::ancestors) {
            if !parent.starts_with(root) {
                break;
            }
            directories.insert(parent.to_owned());
        }
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for directory in directories {
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e)
                    .wrap_err_with(|| format!("retaining directory {}", directory.display()))
            }
        }
    }
    Ok(())
}

fn clean_failure(
    error: eyre::Report,
    directory: &Path,
    owned: &BTreeMap<PathBuf, [u8; 32]>,
) -> eyre::Report {
    match cleanup(directory, owned) {
        Ok(()) => error,
        Err(cleanup) => error.wrap_err(format!("staging cleanup also failed: {cleanup:#}")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
