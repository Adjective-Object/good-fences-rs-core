//! Explicit filesystem observations, retained with the values they produced.
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Missing,
    File,
    Directory,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathInput {
    path: PathBuf,
    kind: InputKind,
    canonical: Option<PathBuf>,
    identity: Option<(u64, u64)>,
    links: Vec<(PathBuf, PathBuf)>,
    content: Option<[u8; 32]>,
}

fn absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn identity(path: &Path, metadata: &fs::Metadata) -> io::Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let _ = metadata;
        let metadata = fs::OpenOptions::new()
            .read(true)
            .access_mode(0)
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS permits directory handles.
            .open(path)?
            .metadata()?;
        Ok((
            metadata
                .volume_serial_number()
                .ok_or_else(|| io::Error::other("missing volume identity"))? as u64,
            metadata
                .file_index()
                .ok_or_else(|| io::Error::other("missing file identity"))?,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, metadata);
        Err(io::Error::other("filesystem identity is unsupported"))
    }
}

impl PathInput {
    /// Reconstruct persisted observations only after checking them against fresh
    /// filesystem reads. This is not an unchecked provenance/content seal.
    pub fn checked_from_parts(
        path: PathBuf,
        kind: InputKind,
        canonical: Option<PathBuf>,
        identity: Option<(u64, u64)>,
        links: Vec<(PathBuf, PathBuf)>,
        content: Option<[u8; 32]>,
    ) -> eyre::Result<Self> {
        eyre::ensure!(path.is_absolute(), "resolution input path is not absolute");
        let input = Self {
            path,
            kind,
            canonical,
            identity,
            links,
            content,
        };
        input.validate()?;
        Ok(input)
    }

    pub fn capture(path: &Path) -> eyre::Result<Self> {
        let path = std::path::absolute(path)?;
        let mut links = Vec::new();
        // Inspect only this path's prefixes, never their siblings or children.
        for prefix in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            match fs::symlink_metadata(prefix) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    links.push((prefix.to_owned(), fs::read_link(prefix)?));
                }
                Ok(_) => {}
                Err(error) if absent(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let (kind, canonical, identity) = match fs::metadata(&path) {
            Ok(metadata) => (
                if metadata.is_file() {
                    InputKind::File
                } else if metadata.is_dir() {
                    InputKind::Directory
                } else {
                    InputKind::Other
                },
                Some(fs::canonicalize(&path)?),
                Some(identity(&path, &metadata)?),
            ),
            Err(error) if absent(&error) => (InputKind::Missing, None, None),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            kind,
            canonical,
            identity,
            links,
            content: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> InputKind {
        self.kind
    }

    pub fn canonical(&self) -> Option<&Path> {
        self.canonical.as_deref()
    }

    pub fn identity(&self) -> Option<(u64, u64)> {
        self.identity
    }

    pub fn links(&self) -> &[(PathBuf, PathBuf)] {
        &self.links
    }

    pub fn content_seal(&self) -> Option<&[u8; 32]> {
        self.content.as_ref()
    }

    /// Seal the exact bytes used by a content-dependent consumer.
    pub fn seal_content(&mut self, bytes: &[u8]) {
        self.content = Some(Sha256::digest(bytes).into());
    }

    pub fn validate(&self) -> eyre::Result<()> {
        let mut fresh = Self::capture(&self.path)?;
        if self.content.is_some() && fresh.kind == InputKind::File {
            fresh.seal_content(&fs::read(&self.path)?);
            let mut after = Self::capture(&self.path)?;
            after.content = fresh.content;
            eyre::ensure!(
                fresh == after,
                "resolution input changed while validating: {}",
                self.path.display()
            );
        }
        eyre::ensure!(
            self == &fresh,
            "resolution input changed: {}",
            self.path.display()
        );
        Ok(())
    }
}

/// Ordered observations, including cached observations at their point of use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputProof {
    inputs: Vec<PathInput>,
    errors: Vec<String>,
}

impl InputProof {
    pub fn inputs(&self) -> &[PathInput] {
        &self.inputs
    }

    pub fn validate(&self) -> eyre::Result<()> {
        self.check_errors()?;
        for input in &self.inputs {
            input.validate()?;
        }
        Ok(())
    }

    pub fn check_errors(&self) -> eyre::Result<()> {
        eyre::ensure!(
            self.errors.is_empty(),
            "unproven resolution inputs: {}",
            self.errors.join("; ")
        );
        Ok(())
    }
}

/// Request-owned, explicit collector. Clones share only this request's capture.
#[derive(Clone, Debug, Default)]
pub struct InputObserver(Arc<Mutex<InputProof>>);

impl InputObserver {
    pub fn record(&self, input: PathInput) {
        self.0.lock().inputs.push(input);
    }

    pub fn error(&self, error: impl std::fmt::Display) {
        self.0.lock().errors.push(error.to_string());
    }

    pub fn replay(&self, proof: &InputProof) {
        let mut capture = self.0.lock();
        capture.inputs.extend_from_slice(&proof.inputs);
        capture.errors.extend_from_slice(&proof.errors);
    }

    pub fn proof(&self) -> InputProof {
        self.0.lock().clone()
    }

    pub fn position(&self) -> (usize, usize) {
        let capture = self.0.lock();
        (capture.inputs.len(), capture.errors.len())
    }

    pub fn since(&self, position: (usize, usize)) -> InputProof {
        let capture = self.0.lock();
        InputProof {
            inputs: capture.inputs[position.0..].to_vec(),
            errors: capture.errors[position.1..].to_vec(),
        }
    }

    /// Preserve legacy bool-probe behavior, but never turn IO errors into valid proofs.
    pub fn is_file(&self, path: &Path) -> bool {
        match PathInput::capture(path) {
            Ok(input) => {
                let result = input.kind() == InputKind::File;
                self.record(input);
                result
            }
            Err(error) => {
                self.error(format!("{}: {error:#}", path.display()));
                path.is_file()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_parts_require_fresh_positive_negative_and_content_validation() {
        let dir = test_tmpdir::test_tmpdir! { "manifest" => "{}" };
        let path = dir.root().join("manifest");
        let mut original = PathInput::capture(&path).unwrap();
        original.seal_content(b"{}");
        let reconstruct = |input: &PathInput| {
            PathInput::checked_from_parts(
                input.path.clone(),
                input.kind,
                input.canonical.clone(),
                input.identity,
                input.links.clone(),
                input.content,
            )
        };
        assert_eq!(reconstruct(&original).unwrap(), original);
        let mut forged = original.clone();
        forged.content = Some([0; 32]);
        assert!(reconstruct(&forged).is_err());
        forged = original.clone();
        forged.identity = Some((0, 0));
        assert!(reconstruct(&forged).is_err());
        forged = original.clone();
        forged.path = PathBuf::from("manifest");
        assert!(reconstruct(&forged).is_err());
        let missing = PathInput::capture(&dir.root().join("missing")).unwrap();
        reconstruct(&missing).unwrap();
        fs::write(missing.path(), "").unwrap();
        assert!(reconstruct(&missing).is_err());
        fs::write(path, "{\"changed\":true}").unwrap();
        assert!(reconstruct(&original).is_err());
    }
}
