use super::*;
use dashmap::DashMap;
use std::{error::Error, fmt, sync::OnceLock};

#[derive(Clone, Debug)]
pub(crate) struct SharedError(Arc<dyn Error + Send + Sync>);

impl SharedError {
    pub(crate) fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(Arc::from(error.into()))
    }
}

impl fmt::Display for SharedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cached filesystem operation failed")
    }
}

impl Error for SharedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

type Cell<T> = Arc<OnceLock<Result<T, SharedError>>>;
type Links = Arc<Vec<(PathBuf, PathBuf)>>;
type Contents = (PathInput, Option<Arc<[u8]>>);

#[derive(Debug, Default)]
struct State {
    paths: DashMap<PathBuf, Cell<PathInput>>,
    prefixes: DashMap<PathBuf, Cell<Links>>,
    contents: DashMap<PathBuf, Cell<Contents>>,
    #[cfg(test)]
    captures: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    prefix_reads: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    content_reads: std::sync::atomic::AtomicUsize,
}

/// One filesystem epoch. Create a separate snapshot for fresh boundary validation.
#[derive(Clone, Debug, Default)]
pub struct InputSnapshot(Arc<State>);

fn initialize<T: Clone>(cell: Cell<T>, load: impl FnOnce() -> eyre::Result<T>) -> eyre::Result<T> {
    cell.get_or_init(|| load().map_err(SharedError::new))
        .clone()
        .map_err(eyre::Report::new)
}

fn cell<T>(map: &DashMap<PathBuf, Cell<T>>, path: &Path) -> Cell<T> {
    map.entry(path.into()).or_default().clone()
}

impl InputSnapshot {
    fn links(&self, path: &Path) -> eyre::Result<Links> {
        initialize(cell(&self.0.prefixes, path), || {
            let parent = path
                .parent()
                .map(|path| self.links(path))
                .transpose()?
                .unwrap_or_default();
            #[cfg(test)]
            self.0
                .prefix_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let target = fs::read_link(path)
                        .wrap_err_with(|| format!("reading input link {}", path.display()))?;
                    let mut links = parent.as_ref().clone();
                    links.push((path.into(), target));
                    Ok(Arc::new(links))
                }

                Ok(_) => Ok(parent),
                Err(error) if absent(&error) => Ok(parent),
                Err(error) => Err(error)
                    .wrap_err_with(|| format!("inspecting input path prefix {}", path.display())),
            }
        })
    }

    pub fn capture(&self, path: &Path) -> eyre::Result<PathInput> {
        let path = std::path::absolute(path)
            .wrap_err_with(|| format!("resolving input observation path {}", path.display()))?;
        initialize(cell(&self.0.paths, &path), || {
            let links = self.links(&path)?;
            #[cfg(test)]
            self.0
                .captures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            PathInput::capture_with_links(path.clone(), links.as_ref().clone())
        })
    }

    /// Retain the exact bytes and original identity together, including missing files.
    pub fn read(&self, path: &Path) -> eyre::Result<Contents> {
        let path = std::path::absolute(path)
            .wrap_err_with(|| format!("resolving content input path {}", path.display()))?;
        initialize(cell(&self.0.contents, &path), || {
            let mut input = self.capture(&path)?;
            if input.kind == InputKind::Missing {
                return Ok((input, None));
            }
            #[cfg(test)]
            self.0
                .content_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let bytes: Arc<[u8]> = fs::read(&path)
                .wrap_err_with(|| format!("reading content input {}", path.display()))?
                .into();
            input.seal_content(&bytes);
            input.validate()?;
            Ok((input, Some(bytes)))
        })
    }

    pub fn validate(&self, input: &PathInput) -> eyre::Result<()> {
        let fresh = if input.content.is_some() {
            self.read(input.path())?.0
        } else {
            self.capture(input.path())?
        };
        eyre::ensure!(
            input == &fresh,
            "resolution input changed: {}",
            input.path().display()
        );
        Ok(())
    }

    pub fn mark_dirty_root(&self, path: &Path) {
        self.0.paths.retain(|key, _| !key.starts_with(path));
        self.0.prefixes.retain(|key, _| !key.starts_with(path));
        self.0.contents.retain(|key, _| !key.starts_with(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, sync::atomic::Ordering};

    #[test]
    fn concurrent_sibling_probes_initialize_each_path_and_prefix_once() {
        let dir = test_tmpdir::test_tmpdir! { "package.json" => "{}" };
        let paths: Vec<_> = (0..64)
            .map(|i| dir.root().join(format!("missing-{i}")))
            .collect();
        let prefixes: BTreeSet<_> = paths.iter().flat_map(|p| p.ancestors()).collect();
        let snapshot = InputSnapshot::default();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for path in &paths {
                        assert_eq!(snapshot.capture(path).unwrap().kind(), InputKind::Missing);
                    }
                });
            }
        });
        assert_eq!(snapshot.0.captures.load(Ordering::Relaxed), paths.len());
        assert_eq!(
            snapshot.0.prefix_reads.load(Ordering::Relaxed),
            prefixes.len()
        );
    }

    #[test]
    fn content_and_proofs_retain_original_bytes_until_a_new_epoch() {
        let dir = test_tmpdir::test_tmpdir! { "package.json" => r#"{"main":"old.js"}"# };
        let path = dir.root().join("package.json");
        let snapshot = InputSnapshot::default();
        let observer = InputObserver::with_snapshot(snapshot.clone());
        let bytes = observer.read(&path).unwrap().unwrap();
        let proof = observer.proof();
        std::fs::write(&path, r#"{"main":"new.js"}"#).unwrap();
        assert_eq!(&*observer.fork().read(&path).unwrap().unwrap(), &*bytes);
        assert_eq!(snapshot.0.content_reads.load(Ordering::Relaxed), 1);
        proof.validate_in(&snapshot).unwrap();
        assert!(proof.validate_in(&InputSnapshot::default()).is_err());
        assert!(proof.validate().is_err());
        snapshot.mark_dirty_root(dir.root());
        assert_eq!(
            &*observer.read(&path).unwrap().unwrap(),
            br#"{"main":"new.js"}"#
        );
    }

    #[test]
    fn invalidation_refreshes_missing_paths_and_conflicting_proofs_are_rejected() {
        let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
        let path = dir.root().join("missing");
        let snapshot = InputSnapshot::default();
        let before = snapshot.capture(&path).unwrap();
        std::fs::write(&path, "").unwrap();
        assert_eq!(snapshot.capture(&path).unwrap(), before);
        snapshot.mark_dirty_root(dir.root());
        let after = snapshot.capture(&path).unwrap();
        assert_eq!(after.kind(), InputKind::File);
        let observer = InputObserver::default();
        observer.record(before);
        observer.record(after);
        assert!(observer.proof().validate_in(&snapshot).is_err());
    }

    #[test]
    fn repeated_read_errors_keep_the_typed_io_cause() {
        let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
        let snapshot = InputSnapshot::default();
        for _ in 0..2 {
            let error = snapshot.read(dir.root()).unwrap_err();
            assert!(error.chain().any(|cause| cause.is::<std::io::Error>()));
            assert!(format!("{error:#}").contains("reading content input"));
        }
        assert_eq!(snapshot.0.content_reads.load(Ordering::Relaxed), 1);
    }
}
