//! Versioned source-local facts; no resolver or graph state.
mod bounded;
mod sidecar;
mod validate;
mod validate_emission;

pub use sidecar::{
    is_cache_artifact, load_or_parse, load_or_parse_with_declarations,
    load_or_parse_with_prepared_declarations, load_prepared, load_prepared_with_declarations,
    sidecar_path, CacheMode, CacheOutcome, CacheStatus, CacheWrite, MissReason, PreparedSource,
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
pub use validate::validate_parsed;

pub const SCHEMA_VERSION: u32 = 12;
pub const EXTRACTOR_VERSION: u32 = 14;
pub const OXC_REVISION: &str = "87f065ebf7cde21d1229322f4c7ee58baca5648e";
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

mod records;
pub use records::{CacheRecord, CachedModule};

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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_builder;

#[cfg(test)]
mod invalidation_tests;
