use std::sync::Arc;

use eyre::{ensure, Result};

use crate::{
    hash128, wire, SharedBytes, CHECKSUM_BYTES, EXTRACTOR_VERSION, MAX_RECORD_BYTES, SCHEMA_VERSION,
};

#[cfg(test)]
#[path = "source_map_tests.rs"]
mod tests;

/// Cold-path construction inputs. Readers retain only verified immutable bytes.
pub struct MapSource<'a> {
    pub original: &'a str,
    pub url: &'a str,
    pub content: Option<&'a str>,
}

pub struct MapData<'a> {
    pub location: &'a str,
    pub file: Option<&'a str>,
    pub source_root: Option<&'a str>,
    pub sources: &'a [MapSource<'a>],
    pub names: &'a [&'a str],
    pub tokens: &'a [wire::MapToken],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireSourceMap {
    bytes: SharedBytes,
}

impl WireSourceMap {
    /// Move already verified bytes to private disk backing without unpacking.
    pub fn snapshot(self) -> Result<Self> {
        if self.bytes.is_mapped() {
            return Ok(self);
        }
        Ok(Self {
            bytes: SharedBytes::snapshot(self.bytes.as_ref())?,
        })
    }

    pub fn encode(source: &str, input: &[u8], data: MapData<'_>) -> Result<Self> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let sources = data
            .sources
            .iter()
            .map(|source| {
                let original = builder.create_string(source.original);
                let url = builder.create_string(source.url);
                let content = source.content.map(|content| builder.create_string(content));
                wire::MapSource::create(
                    &mut builder,
                    &wire::MapSourceArgs {
                        original: Some(original),
                        url: Some(url),
                        content,
                    },
                )
            })
            .collect::<Vec<_>>();
        let sources = builder.create_vector(&sources);
        let names = data
            .names
            .iter()
            .map(|name| builder.create_string(name))
            .collect::<Vec<_>>();
        let names = builder.create_vector(&names);
        let tokens = builder.create_vector(data.tokens);
        let location = builder.create_string(data.location);
        let file = data.file.map(|file| builder.create_string(file));
        let source_root = data.source_root.map(|root| builder.create_string(root));
        let root = wire::UpstreamMap::create(
            &mut builder,
            &wire::UpstreamMapArgs {
                schema: SCHEMA_VERSION,
                extractor: EXTRACTOR_VERSION,
                source_hash: Some(&wire::Hash128::new(&hash128(source.as_bytes()))),
                input_hash: Some(&wire::Hash128::new(&hash128(input))),
                location: Some(location),
                file,
                source_root,
                sources: Some(sources),
                names: Some(names),
                tokens: Some(tokens),
            },
        );
        builder.finish(root, Some("GFUM"));
        let mut bytes = builder.finished_data().to_vec();
        bytes.extend_from_slice(&hash128(builder.finished_data()));
        Self::new(bytes.into(), source, input, data.location)
    }

    pub fn new(bytes: Arc<[u8]>, source: &str, input: &[u8], location: &str) -> Result<Self> {
        Self::from_backing(bytes.into(), source, input, location)
    }

    pub fn from_backing(
        bytes: SharedBytes,
        source: &str,
        input: &[u8],
        location: &str,
    ) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES && bytes.len() >= 8 + CHECKSUM_BYTES,
            "invalid source map cache size"
        );
        let (payload, checksum) = bytes.split_at(bytes.len() - CHECKSUM_BYTES);
        ensure!(
            hash128(payload).as_slice() == checksum,
            "source map cache checksum mismatch"
        );
        ensure!(
            flatbuffers::buffer_has_identifier(payload, "GFUM", false),
            "invalid source map cache identifier"
        );
        let root = flatbuffers::root_with_opts::<wire::UpstreamMap<'_>>(
            &crate::options(MAX_RECORD_BYTES),
            payload,
        )?;
        ensure!(
            root.schema() == SCHEMA_VERSION && root.extractor() == EXTRACTOR_VERSION,
            "source map cache version mismatch"
        );
        ensure!(
            root.source_hash()
                .bytes()
                .iter()
                .eq(hash128(source.as_bytes()))
                && root.input_hash().bytes().iter().eq(hash128(input))
                && root.location() == location,
            "source map cache provenance mismatch"
        );
        let mut previous = None;
        for token in root.tokens() {
            let key = (token.generated_line(), token.generated_column());
            ensure!(
                previous.is_none_or(|p| p <= key),
                "unordered source map tokens"
            );
            previous = Some(key);
            ensure!(
                token.source() >= -1
                    && token.name() >= -1
                    && (token.source() == -1 || (token.source() as usize) < root.sources().len())
                    && (token.name() == -1 || (token.name() as usize) < root.names().len())
                    && (token.source() != -1 || token.name() == -1),
                "invalid source map token reference"
            );
        }
        Ok(Self { bytes })
    }

    pub fn root(&self) -> wire::UpstreamMap<'_> {
        // SAFETY: construction verifies these exact bytes, including every
        // nested table and semantic index; no mutable aliases are exposed.
        unsafe {
            flatbuffers::root_unchecked::<wire::UpstreamMap<'_>>(
                &self.bytes[..self.bytes.len() - CHECKSUM_BYTES],
            )
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn backing(&self) -> &SharedBytes {
        &self.bytes
    }

    pub fn matches_inputs(&self, source: &str, input: &[u8], location: &str) -> bool {
        let root = self.root();
        root.source_hash()
            .bytes()
            .iter()
            .eq(hash128(source.as_bytes()))
            && root.input_hash().bytes().iter().eq(hash128(input))
            && root.location() == location
    }

    /// Greatest lower bound on the same generated line; unmapped tokens remain
    /// explicit boundaries and never fall through to an earlier mapped token.
    pub fn lookup(&self, line: u32, column: u32) -> Option<&wire::MapToken> {
        let tokens = self.root().tokens();
        let mut low = 0;
        let mut high = tokens.len();
        while low < high {
            let middle = low + (high - low) / 2;
            let token = tokens.get(middle);
            if (token.generated_line(), token.generated_column()) <= (line, column) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let token = tokens.get(low.checked_sub(1)?);
        (token.generated_line() == line).then_some(token)
    }
}
