//! Typed source-cache wire format. This crate never owns a decoded syntax tree.
#[rustfmt::skip]
#[expect(
    unused_imports,
    non_snake_case,
    mismatched_lifetime_syntaxes,
    clippy::extra_unused_lifetimes,
    clippy::derivable_impls,
    clippy::missing_safety_doc,
    clippy::unnecessary_cast,
    reason = "unmodified flatc 25.2.10 output"
)]
mod cache_generated;
pub use cache_generated::source_cache as wire;

#[rustfmt::skip]
#[expect(
    unused_imports,
    mismatched_lifetime_syntaxes,
    clippy::extra_unused_lifetimes,
    clippy::derivable_impls,
    clippy::missing_safety_doc,
    reason = "unmodified flatc 25.2.10 output"
)]
mod resolution_generated;
pub use resolution_generated::resolution_cache as resolution_wire;
pub mod resolution;

mod source_map;
mod verify;
pub use source_map::{MapData, MapSource, WireSourceMap};

use std::{ops::Range, sync::Arc};

use eyre::{ensure, Result};

pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
pub const SCHEMA_VERSION: u32 = 21;
pub const EXTRACTOR_VERSION: u32 = 34;
pub const MODULE_IDENTIFIER: &str = "GFSM";
const CHECKSUM_BYTES: usize = 16;

/// XXH3-128 in the same big-endian byte order as internal fingerprints.
/// Detects accidental corruption, not authentication of cache publishers.
pub fn hash128(bytes: &[u8]) -> [u8; 16] {
    xxhash_rust::xxh3::xxh3_128(bytes).to_be_bytes()
}

/// Check Ecco discriminants, canonical maps and host ID widths after generated
/// structural verification, including when Ecco is embedded in another root.
pub fn validate_ecco(root: wire::ParsedFile<'_>) -> Result<()> {
    verify::ecco(root)
}

/// Detection policy is an input; the effective format is sealed extraction output.
pub fn parser_config_matches(
    requested: &wire::ParserConfig,
    effective: &wire::ParserConfig,
) -> bool {
    requested.typescript() == effective.typescript()
        && requested.jsx() == effective.jsx()
        && requested.definition() == effective.definition()
        && requested.detect_esm() == effective.detect_esm()
        && (requested.detect_esm() || requested.commonjs() == effective.commonjs())
}

fn options() -> flatbuffers::VerifierOptions {
    flatbuffers::VerifierOptions {
        max_depth: 64,
        max_tables: 2_097_152,
        max_apparent_size: MAX_RECORD_BYTES * 4,
        ..Default::default()
    }
}

/// Independently verified Ecco buffer, useful before a full module is sealed.
/// Production nested Ecco tables instead borrow their verified module owner.
#[derive(Clone, Debug)]
pub struct WireEcco {
    bytes: Arc<[u8]>,
}

impl WireEcco {
    pub fn new(bytes: Arc<[u8]>) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "Ecco buffer exceeds byte limit"
        );
        let root = flatbuffers::root_with_opts::<wire::ParsedFile<'_>>(&options(), &bytes)?;
        verify::ecco(root)?;
        Ok(Self { bytes })
    }

    pub fn root(&self) -> wire::ParsedFile<'_> {
        // SAFETY: construction verifies the entire table graph and the bytes
        // remain immutable, including across clones of this owner.
        unsafe { flatbuffers::root_unchecked::<wire::ParsedFile<'_>>(&self.bytes) }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
/// Immutable, structurally verified bytes with source/configuration integrity.
///
/// This is not the source-local semantic validation gate: callers must also
/// validate spans, occurrence links, ordering, and supported emission recipes.
/// No mutable byte access, unpacking, or self-referential references are retained.
#[derive(Clone, Debug)]
pub struct WireRecord {
    bytes: Arc<[u8]>,
    payload_end: usize,
    module: Range<usize>,
}

impl WireRecord {
    pub fn new(
        bytes: Arc<[u8]>,
        source: &str,
        config: wire::ParserConfig,
        oxc: &str,
    ) -> Result<Self> {
        ensure!(bytes.len() <= MAX_RECORD_BYTES, "cache exceeds byte limit");
        let payload_end = bytes
            .len()
            .checked_sub(CHECKSUM_BYTES)
            .ok_or_else(|| eyre::eyre!("truncated cache checksum"))?;
        let (payload, checksum) = bytes.split_at(payload_end);
        ensure!(
            hash128(payload).as_slice() == checksum,
            "cache checksum mismatch"
        );
        let root = verified_root(payload)?;
        ensure!(
            root.schema() == SCHEMA_VERSION
                && root.extractor() == EXTRACTOR_VERSION
                && root.oxc() == oxc,
            "cache version mismatch"
        );
        let provenance = root.provenance();
        let source_hash = hash128(source.as_bytes());
        ensure!(
            source.len() <= u32::MAX as usize
                && root.source_len() == source.len() as u64
                && root.source_hash().bytes().iter().eq(source_hash)
                && provenance.source_hash().bytes().iter().eq(source_hash),
            "cache source mismatch"
        );
        ensure!(
            parser_config_matches(&config, root.config())
                && provenance.config() == root.config()
                && (!config.definition() || config.typescript()),
            "cache parser configuration mismatch"
        );
        let config = root.config();
        let module_bytes = root.module().bytes();
        let records_hash = hash128(module_bytes);
        ensure!(
            provenance.records_hash().bytes().iter().eq(records_hash),
            "cache extraction provenance mismatch"
        );
        let module_root = verified_module(module_bytes)?;
        verify::module(module_root)?;
        ensure!(
            module_root.facts().commonjs() == config.commonjs()
                && (module_root.body_type() == wire::ModuleBody::CommonJsBody) == config.commonjs()
                && module_root.facts().ecco().is_none() == config.commonjs(),
            "cache module format mismatch"
        );
        // The vector was verified above and is borrowed from `payload`.
        let start = module_bytes.as_ptr() as usize - payload.as_ptr() as usize;
        let module = start..start + module_bytes.len();
        Ok(Self {
            bytes,
            payload_end,
            module,
        })
    }

    pub fn root(&self) -> wire::CacheRecord<'_> {
        // SAFETY: new verifies these exact immutable bytes before constructing
        // the owner. Clones share the allocation, never a mutable alias.
        unsafe {
            wire::size_prefixed_root_as_cache_record_unchecked(&self.bytes[..self.payload_end])
        }
    }

    pub fn module(&self) -> wire::CachedModule<'_> {
        // SAFETY: new independently verifies the nested root and records its
        // checked vector range. The returned view cannot outlive this owner.
        unsafe {
            flatbuffers::root_unchecked::<wire::CachedModule<'_>>(&self.bytes[self.module.clone()])
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn verified_root(bytes: &[u8]) -> Result<wire::CacheRecord<'_>> {
    ensure!(
        bytes.len() >= 12 && wire::cache_record_size_prefixed_buffer_has_identifier(bytes),
        "invalid cache FlatBuffer identifier"
    );
    ensure!(
        u32::from_le_bytes(bytes[..4].try_into()?) as usize == bytes.len() - 4,
        "cache size prefix mismatch"
    );
    Ok(wire::size_prefixed_root_as_cache_record_with_opts(
        &options(),
        bytes,
    )?)
}

fn verified_module(bytes: &[u8]) -> Result<wire::CachedModule<'_>> {
    ensure!(
        bytes.len() >= 8 && flatbuffers::buffer_has_identifier(bytes, MODULE_IDENTIFIER, false),
        "invalid module FlatBuffer identifier"
    );
    Ok(flatbuffers::root_with_opts::<wire::CachedModule<'_>>(
        &options(),
        bytes,
    )?)
}

/// Append the content checksum to a finished typed record. Construction only;
/// readers never rebuild or re-encode the record to validate canonical bytes.
pub fn frame(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        payload.len() <= MAX_RECORD_BYTES - CHECKSUM_BYTES,
        "cache exceeds byte limit"
    );
    let root = verified_root(payload)?;
    verify::module(verified_module(root.module().bytes())?)?;
    let mut bytes = Vec::with_capacity(payload.len() + CHECKSUM_BYTES);
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&hash128(payload));
    Ok(bytes)
}

#[cfg(test)]
mod tests;
