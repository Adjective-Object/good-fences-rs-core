//! Versioned source-local facts; no resolver or graph state.
mod bounded;
mod sidecar;
mod validate;
mod validate_emission;

pub use sidecar::{
    is_cache_artifact, load_or_parse, load_or_parse_with_declarations, load_prepared, sidecar_path,
    CacheMode, CacheOutcome, CacheStatus, CacheWrite, MissReason, PreparedSource,
};

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use ast_segmenter::ModuleFormat;
pub use ast_segmenter::ParserConfig;
use ast_segmenter::{owned::ModuleFacts, ModuleBody, ParsedModule};
use eyre::ensure;
use oxc_span::Span;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 10;
pub const EXTRACTOR_VERSION: u32 = 13;
pub const OXC_REVISION: &str = "87f065ebf7cde21d1229322f4c7ee58baca5648e";
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Projection of ParsedModule that preserves executable occurrence facts and
/// statement ranges without serializing legacy tracker tags or name sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModule {
    pub body: ModuleBody,
    #[serde(with = "ast_segmenter::owned::span_serde::list")]
    pub segments: Vec<Span>,
    pub facts: ModuleFacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRecord {
    magic: String,
    schema: u32,
    extractor: u32,
    oxc: String,
    pub source_len: u64,
    pub source_hash: [u8; 32],
    pub config: ParserConfig,
    pub module: CachedModule,
    provenance: ast_segmenter::SourceProvenance,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    checksum: [u8; 32],
    #[serde(with = "payload_bytes")]
    payload: Vec<u8>,
}

mod payload_bytes {
    use serde::{de::Visitor, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct Bytes;
        impl Visitor<'_> for Bytes {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("CBOR byte string")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(v)
            }
        }
        d.deserialize_byte_buf(Bytes)
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl CacheRecord {
    pub fn matches_declarations(&self, declarations: &ecco::LocalFacts) -> bool {
        ast_segmenter::owned::declarations_hash(declarations)
            .is_ok_and(|hash| hash == self.module.facts.ecco_declarations_hash)
    }

    pub fn from_parsed(source: &str, parsed: &ParsedModule) -> eyre::Result<Self> {
        let record = Self {
            magic: "good-fences-segments".into(),
            schema: SCHEMA_VERSION,
            extractor: EXTRACTOR_VERSION,
            oxc: OXC_REVISION.into(),
            source_len: source.len() as u64,
            source_hash: digest(source.as_bytes()),
            config: ParserConfig::from_source_type(parsed.source_type),
            module: CachedModule {
                body: parsed.body,
                segments: parsed.segments.iter().map(|s| s.span).collect(),
                facts: parsed.facts.clone(),
            },
            provenance: parsed.provenance().clone(),
        };
        record.validate(source, record.config)?;
        Ok(record)
    }

    /// Checked cache adapter shared by graph construction and emission.
    pub fn to_parsed(&self, source: &str) -> eyre::Result<ParsedModule> {
        self.validate(source, self.config)?;
        ParsedModule::from_provenance(
            source,
            self.provenance.clone(),
            self.module.body,
            self.module
                .segments
                .iter()
                .map(|&span| ast_segmenter::Segment {
                    span,
                    module_deps: Default::default(),
                    variables: Default::default(),
                })
                .collect(),
            self.module.facts.clone(),
        )
    }

    pub fn encode(&self, source: &str) -> eyre::Result<Vec<u8>> {
        self.validate(source, self.config)?;
        let payload = serde_cbor::to_vec(self)?;
        bounded::check(&payload)?;
        let bytes = serde_cbor::to_vec(&Envelope {
            checksum: digest(&payload),
            payload,
        })?;
        bounded::check(&bytes)?;
        Ok(bytes)
    }

    /// Invalid/stale records are errors here; filesystem callers treat them as misses.
    pub fn decode(bytes: &[u8], source: &str, config: ParserConfig) -> eyre::Result<Self> {
        bounded::check(bytes)?;
        let envelope: Envelope = serde_cbor::from_slice(bytes)?;
        ensure!(
            digest(&envelope.payload) == envelope.checksum,
            "cache checksum mismatch"
        );
        bounded::check(&envelope.payload)?;
        let record: Self = serde_cbor::from_slice(&envelope.payload)?;
        record.validate(source, config)?;
        ensure!(record.encode(source)? == bytes, "noncanonical cache record");
        Ok(record)
    }

    pub fn decode_with_declarations(
        bytes: &[u8],
        source: &str,
        config: ParserConfig,
        declarations: &ecco::LocalFacts,
    ) -> eyre::Result<Self> {
        let record = Self::decode(bytes, source, config)?;
        ensure!(
            record.matches_declarations(declarations),
            "cache Ecco declaration configuration mismatch"
        );
        Ok(record)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod invalidation_tests;
