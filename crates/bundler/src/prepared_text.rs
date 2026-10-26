//! Checked input identities and immutable, disk-backed prepared text leases.
use std::{ops::Deref, path::Path};

use ast_segmenter::fingerprint::Fingerprint;
use cache_flatbuffers::SharedBytes;
use eyre::{ensure, WrapErr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InputIdentity {
    len: usize,
    hash: Fingerprint,
}

impl InputIdentity {
    pub(crate) fn new(bytes: &[u8]) -> Self {
        Self {
            len: bytes.len(),
            hash: Fingerprint::of(bytes),
        }
    }

    pub(crate) fn matches(self, bytes: &[u8]) -> bool {
        self == Self::new(bytes)
    }

    pub(crate) fn read(self, path: &Path) -> eyre::Result<Vec<u8>> {
        let bytes = std::fs::read(path)
            .wrap_err_with(|| format!("rereading prepared input {}", path.display()))?;
        ensure!(
            self.matches(&bytes),
            "source changed during build: {}",
            path.display()
        );
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Text(SharedBytes);

impl Text {
    pub(crate) fn snapshot(text: &str) -> eyre::Result<Self> {
        Ok(Self(SharedBytes::snapshot(text.as_bytes())?))
    }

    pub(crate) fn read(bytes: &[u8], path: &Path) -> eyre::Result<Self> {
        let text = std::str::from_utf8(bytes)
            .wrap_err_with(|| format!("reading UTF-8 prepared input {}", path.display()))?;
        Self::snapshot(text)
            .wrap_err_with(|| format!("snapshotting prepared input {}", path.display()))
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: constructors accept only checked UTF-8 and SharedBytes has no
        // writable aliases. Revalidating here would rescan a file per source span.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedText {
    identity: InputIdentity,
    generated: Option<Text>,
}

impl PreparedText {
    pub(crate) fn new(input: &[u8], text: &str, generated: bool) -> eyre::Result<Self> {
        Ok(Self {
            identity: InputIdentity::new(input),
            generated: (generated || input != text.as_bytes())
                .then(|| Text::snapshot(text))
                .transpose()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn generated_backing(&self) -> Option<&SharedBytes> {
        self.generated.as_ref().map(|text| &text.0)
    }

    pub(crate) fn matches(&self, input: &[u8], text: &str) -> bool {
        self.identity.matches(input)
            && match &self.generated {
                Some(old) => &**old == text,
                None => input == text.as_bytes(),
            }
    }

    pub(crate) fn matches_input(&self, input: &[u8]) -> bool {
        self.identity.matches(input)
    }

    pub(crate) fn read_input(&self, path: &Path) -> eyre::Result<Vec<u8>> {
        self.identity.read(path)
    }

    pub(crate) fn text(&self, path: &Path) -> eyre::Result<Text> {
        let input = self.read_input(path)?;
        match &self.generated {
            Some(text) => Ok(text.clone()),
            None => Text::read(&input, path),
        }
    }
}
