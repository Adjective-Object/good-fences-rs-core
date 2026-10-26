use std::path::Path;

use cache_flatbuffers::hash128;
use eyre::ensure;
use oxc_span::{SourceType, Span};
use serde::{Deserialize, Serialize};

use crate::{owned::ModuleFacts, ModuleBody, ModuleFormat};

/// Effective parser policy, independent of package discovery and source paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserConfig {
    typescript: bool,
    jsx: bool,
    definition: bool,
    commonjs: bool,
    detect_esm: bool,
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

    pub fn detect_esm(&self) -> bool {
        self.detect_esm
    }

    /// Match a requested policy without recomputing its cached syntax decision.
    pub fn accepts(&self, effective: Self) -> bool {
        cache_flatbuffers::parser_config_matches(&self.wire(), &effective.wire())
    }

    pub fn resolved(mut self, source_type: SourceType) -> Self {
        self.commonjs = source_type.is_commonjs();
        self
    }
}

impl ParserConfig {
    pub fn for_path(path: &Path, package: ModuleFormat) -> eyre::Result<Self> {
        let mut source_type = oxc_utils_parse::bundler_source_type(path)?;
        let detect_esm = source_type.is_unambiguous() && package == ModuleFormat::CommonJs;
        if source_type.is_unambiguous() {
            source_type = match package {
                ModuleFormat::Esm => source_type.with_module(true),
                ModuleFormat::CommonJs => source_type.with_commonjs(true),
            };
        }
        let mut config = Self::from_source_type(source_type);
        config.detect_esm = detect_esm;
        Ok(config)
    }

    pub fn from_source_type(source_type: SourceType) -> Self {
        Self {
            typescript: source_type.is_typescript(),
            jsx: source_type.is_jsx(),
            definition: source_type.is_typescript_definition(),
            commonjs: source_type.is_commonjs(),
            detect_esm: false,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceProvenance {
    source_hash: [u8; 16],
    config: ParserConfig,
    records_hash: [u8; 16],
    #[serde(skip)]
    wire: Option<cache_flatbuffers::WireRecord>,
}

impl PartialEq for SourceProvenance {
    fn eq(&self, other: &Self) -> bool {
        self.source_hash == other.source_hash
            && self.config == other.config
            && self.records_hash == other.records_hash
            && self.wire.is_some() == other.wire.is_some()
    }
}
impl Eq for SourceProvenance {}

fn records_hash(body: ModuleBody, ranges: &[Span], facts: &ModuleFacts) -> eyre::Result<[u8; 16]> {
    Ok(hash128(&facts.encode_module(body, ranges)))
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
            source_hash: hash128(source.as_bytes()),
            config,
            records_hash: records_hash(body, ranges, facts)?,
            wire: None,
        })
    }

    pub fn config(&self) -> ParserConfig {
        self.config
    }

    pub(crate) fn wire_record(&self) -> Option<&cache_flatbuffers::WireRecord> {
        self.wire.as_ref()
    }

    pub fn verify(
        &self,
        source: &str,
        config: ParserConfig,
        body: ModuleBody,
        ranges: &[Span],
        facts: crate::views::ModuleFacts<'_>,
    ) -> eyre::Result<()> {
        ensure!(
            self.source_hash == hash128(source.as_bytes()),
            "source provenance content mismatch"
        );
        ensure!(
            self.config == config,
            "source parser configuration mismatch"
        );
        match facts {
            crate::views::ModuleFacts::Owned(facts) => ensure!(
                self.wire.is_none() && self.records_hash == records_hash(body, ranges, facts)?,
                "source facts/ranges do not match extraction provenance"
            ),
            crate::views::ModuleFacts::Wire(facts) => {
                let wire = self
                    .wire
                    .as_ref()
                    .ok_or_else(|| eyre::eyre!("wire facts require wire extraction provenance"))?;
                let module = wire.module();
                ensure!(
                    std::ptr::eq(module.facts()._tab.buf(), facts._tab.buf())
                        && module.facts()._tab.loc() == facts._tab.loc()
                        && crate::views::module_body(module) == body
                        && module
                            .segments()
                            .iter()
                            .map(crate::views::span)
                            .eq(ranges.iter().copied()),
                    "wire module/ranges do not match extraction provenance"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn from_wire(
        source: &str,
        wire: &cache_flatbuffers::WireRecord,
    ) -> eyre::Result<Self> {
        let root = wire.root();
        let p = root.provenance();
        let c = p.config();
        let result = Self {
            source_hash: std::array::from_fn(|i| p.source_hash().bytes().get(i)),
            config: ParserConfig {
                typescript: c.typescript(),
                jsx: c.jsx(),
                definition: c.definition(),
                commonjs: c.commonjs(),
                detect_esm: c.detect_esm(),
            },
            records_hash: std::array::from_fn(|i| p.records_hash().bytes().get(i)),
            wire: Some(wire.clone()),
        };
        ensure!(
            result.source_hash == hash128(source.as_bytes()),
            "source provenance content mismatch"
        );
        Ok(result)
    }
}
