//! Versioned source-local facts; no resolver or graph state.
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
use ast_segmenter::{ModuleBody, ParsedModule};
use eyre::ensure;
use oxc_span::Span;
use sha2::{Digest, Sha256};
pub use validate::validate_parsed;

pub use cache_flatbuffers::{EXTRACTOR_VERSION, MAX_RECORD_BYTES, SCHEMA_VERSION};
pub const OXC_REVISION: &str = "87f065ebf7cde21d1229322f4c7ee58baca5648e";

mod records;
pub use records::{CacheRecord, CachedModule};

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_builder;

#[cfg(test)]
mod invalidation_tests;
