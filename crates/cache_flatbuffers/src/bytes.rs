use std::{fs::File, io::Read, ops::Deref, path::Path, sync::Arc};

use eyre::{Result, WrapErr};

/// Immutable shared storage. Heap storage is for low-level construction adapters;
/// production cache readers snapshot into private disk-backed storage.
#[derive(Clone, Debug)]
pub struct SharedBytes(Storage);

#[derive(Clone, Debug)]
enum Storage {
    Heap(Arc<[u8]>),
    Mapped(Arc<MappedBytes>),
}

#[derive(Debug)]
struct MappedBytes {
    // Fields drop in declaration order: unmap before closing/deleting on Windows.
    map: Option<memmap2::Mmap>,
    _file: File,
}

impl SharedBytes {
    /// Stream into a private file in the OS temporary directory, then map read-only.
    /// Never maps a caller's mutable file, even after a successful checksum.
    pub fn snapshot(reader: impl Read) -> Result<Self> {
        Self::snapshot_in(&std::env::temp_dir(), reader)
    }

    /// Like `snapshot`, with an explicit writable backing-file directory.
    /// No filename, writable handle or mutable mapping escapes this constructor.
    pub fn snapshot_in(directory: &Path, mut reader: impl Read) -> Result<Self> {
        let mut file = private_file(directory).wrap_err_with(|| {
            format!("creating private byte snapshot in {}", directory.display())
        })?;
        let len = std::io::copy(&mut reader, &mut file).wrap_err_with(|| {
            format!("writing private byte snapshot in {}", directory.display())
        })?;
        let map = if len == 0 {
            None
        } else {
            // SAFETY: this file is unlinked on Unix, or exclusively opened with
            // delete-on-close on Windows. It has no external writable aliases.
            // All writes finished above; the only handle is retained privately,
            // never used again, and dropped only after this read-only mapping.
            Some(
                unsafe { memmap2::MmapOptions::new().map(&file) }.wrap_err_with(|| {
                    format!("mapping private byte snapshot in {}", directory.display())
                })?,
            )
        };
        Ok(Self(Storage::Mapped(Arc::new(MappedBytes {
            map,
            _file: file,
        }))))
    }

    pub fn is_mapped(&self) -> bool {
        matches!(self.0, Storage::Mapped(_))
    }
}

#[cfg(unix)]
fn private_file(directory: &Path) -> Result<File> {
    let (file, path) = tempfile::NamedTempFile::new_in(directory)?.into_parts();
    // Unlike an unchecked best-effort unlink, failure must prevent mapping.
    path.close()?;
    Ok(file)
}

#[cfg(windows)]
fn private_file(directory: &Path) -> Result<File> {
    // tempfile uses share_mode(0) and FILE_FLAG_DELETE_ON_CLOSE.
    Ok(tempfile::tempfile_in(directory)?)
}

#[cfg(not(any(unix, windows)))]
fn private_file(_directory: &Path) -> Result<File> {
    eyre::bail!("private file-backed byte snapshots require Unix or Windows")
}

impl From<Arc<[u8]>> for SharedBytes {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self(Storage::Heap(bytes))
    }
}

impl Deref for SharedBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            Storage::Heap(bytes) => bytes,
            Storage::Mapped(bytes) => bytes.map.as_deref().unwrap_or_default(),
        }
    }
}

impl PartialEq for SharedBytes {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for SharedBytes {}

#[cfg(test)]
mod tests;
