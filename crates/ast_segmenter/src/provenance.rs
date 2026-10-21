use std::path::Path;

use eyre::ensure;
use oxc_span::{SourceType, Span};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{owned::ModuleFacts, ModuleBody, ModuleFormat};

/// Effective parser policy, independent of package discovery and source paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserConfig {
    typescript: bool,
    jsx: bool,
    definition: bool,
    commonjs: bool,
}

impl ParserConfig {
    pub fn typescript(&self) -> bool {
        self.typescript
    }

    pub fn jsx(&self) -> bool {
        self.jsx
    }

    pub fn definition(&self) -> bool {
        self.definition
    }

    pub fn commonjs(&self) -> bool {
        self.commonjs
    }
}

impl ParserConfig {
    pub fn for_path(path: &Path, package: ModuleFormat) -> eyre::Result<Self> {
        let mut source_type = SourceType::from_path(path)?;
        if source_type.is_unambiguous() {
            source_type = match package {
                ModuleFormat::Esm => source_type.with_module(true),
                ModuleFormat::CommonJs => source_type.with_commonjs(true),
            };
        }
        Ok(Self::from_source_type(source_type))
    }

    pub fn from_source_type(source_type: SourceType) -> Self {
        Self {
            typescript: source_type.is_typescript(),
            jsx: source_type.is_jsx(),
            definition: source_type.is_typescript_definition(),
            commonjs: source_type.is_commonjs(),
        }
    }

    pub fn source_type(self) -> SourceType {
        SourceType::default()
            .with_typescript(self.typescript)
            .with_jsx(self.jsx)
            .with_typescript_definition(self.definition)
            .with_commonjs(self.commonjs)
            .with_module(!self.commonjs)
    }
}

/// Extraction integrity, not authentication of a cache publisher.
/// Private fields prevent accidentally pairing arbitrary facts with source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceProvenance {
    source_hash: [u8; 32],
    config: ParserConfig,
    records_hash: [u8; 32],
}

fn records_hash(body: ModuleBody, ranges: &[Span], facts: &ModuleFacts) -> eyre::Result<[u8; 32]> {
    let ranges: Vec<_> = ranges.iter().map(|s| (s.start, s.end)).collect();
    Ok(Sha256::digest(serde_cbor::to_vec(&(body, ranges, facts))?).into())
}

impl SourceProvenance {
    pub(crate) fn extracted(
        source: &str,
        config: ParserConfig,
        body: ModuleBody,
        ranges: &[Span],
        facts: &ModuleFacts,
    ) -> eyre::Result<Self> {
        Ok(Self {
            source_hash: Sha256::digest(source.as_bytes()).into(),
            config,
            records_hash: records_hash(body, ranges, facts)?,
        })
    }

    pub fn config(&self) -> ParserConfig {
        self.config
    }

    pub fn verify(
        &self,
        source: &str,
        config: ParserConfig,
        body: ModuleBody,
        ranges: &[Span],
        facts: &ModuleFacts,
    ) -> eyre::Result<()> {
        ensure!(
            self.source_hash == <[u8; 32]>::from(Sha256::digest(source.as_bytes())),
            "source provenance content mismatch"
        );
        ensure!(
            self.config == config,
            "source parser configuration mismatch"
        );
        ensure!(
            self.records_hash == records_hash(body, ranges, facts)?,
            "source facts/ranges do not match extraction provenance"
        );
        Ok(())
    }
}
