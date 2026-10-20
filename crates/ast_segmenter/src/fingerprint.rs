//! Non-cryptographic internal identities, separate from source/content integrity.
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint([u8; 16]);

impl Fingerprint {
    pub fn of(bytes: &[u8]) -> Self {
        Self(xxhash_rust::xxh3::xxh3_128(bytes).to_be_bytes())
    }
}

/// Identity of effective declaration facts, not declaration source text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeclarationFingerprint(Fingerprint);

thread_local! {
    static DECLARATION_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Declaration serializations/hashes on this thread, including compatibility APIs.
pub fn declaration_fingerprint_count() -> usize {
    DECLARATION_HASHES.with(std::cell::Cell::get)
}

pub(crate) fn declarations_hash(facts: &ecco::LocalFacts) -> eyre::Result<DeclarationFingerprint> {
    let bytes = serde_cbor::to_vec(facts)?;
    DECLARATION_HASHES.with(|count| count.set(count.get() + 1));
    Ok(DeclarationFingerprint(Fingerprint::of(&bytes)))
}

/// Immutable, validated contracts and their computed identity. No caller-supplied
/// digest or mutable facts can be associated with this prepared configuration.
#[derive(Debug, Clone)]
pub struct PreparedDeclarations {
    facts: Arc<ecco::LocalFacts>,
    fingerprint: DeclarationFingerprint,
}

impl PreparedDeclarations {
    pub fn new(facts: ecco::LocalFacts) -> eyre::Result<Self> {
        ecco::Context::from_local_facts(facts.clone())?;
        let fingerprint = declarations_hash(&facts)?;
        Ok(Self {
            facts: Arc::new(facts),
            fingerprint,
        })
    }

    pub fn from_config(config: &ecco::DeclarationConfig) -> eyre::Result<Self> {
        Self::new(config.effective()?)
    }

    pub fn facts(&self) -> &ecco::LocalFacts {
        &self.facts
    }

    pub fn fingerprint(&self) -> DeclarationFingerprint {
        self.fingerprint
    }
}
