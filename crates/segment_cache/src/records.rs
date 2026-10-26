use std::sync::{Arc, OnceLock};

use super::*;

/// Borrowed syntax and lazy scalar ranges backed by a verified immutable buffer.
#[derive(Debug, Clone)]
pub struct CachedModule {
    parsed: ParsedModule,
    segments: OnceLock<Vec<Span>>,
}

impl CachedModule {
    pub fn body(&self) -> ModuleBody {
        self.parsed.body()
    }

    pub fn segments(&self) -> &[Span] {
        self.segments
            .get_or_init(|| self.parsed.segments().iter().map(|s| s.span).collect())
    }

    pub fn facts(&self) -> ast_segmenter::views::ModuleFacts<'_> {
        self.parsed.facts()
    }
}

/// Construction is the semantic validation gate. Clones share the checked
/// module and wire allocation; no public constructor accepts an unchecked seal.
#[derive(Debug, Clone)]
pub struct CacheRecord {
    wire: Option<cache_flatbuffers::WireRecord>,
    module: Arc<CachedModule>,
    source_len: u64,
    source_hash: [u8; 16],
    write_error: Option<Arc<str>>,
}

impl PartialEq for CacheRecord {
    fn eq(&self, other: &Self) -> bool {
        match (&self.wire, &other.wire) {
            (Some(a), Some(b)) => a.bytes() == b.bytes(),
            (None, None) => {
                self.source_len == other.source_len && self.provenance() == other.provenance()
            }
            _ => false,
        }
    }
}
impl Eq for CacheRecord {}

impl CacheRecord {
    #[cfg(test)]
    pub(crate) fn wire_bytes_for_test(&self) -> &[u8] {
        self.wire.as_ref().unwrap().bytes()
    }

    pub fn schema(&self) -> u32 {
        SCHEMA_VERSION
    }

    pub fn extractor(&self) -> u32 {
        EXTRACTOR_VERSION
    }

    pub fn oxc(&self) -> &str {
        OXC_REVISION
    }

    pub fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn source_hash(&self) -> [u8; 16] {
        self.source_hash
    }

    pub fn config(&self) -> ParserConfig {
        self.provenance().config()
    }

    pub fn module(&self) -> &CachedModule {
        &self.module
    }

    pub fn provenance(&self) -> &ast_segmenter::SourceProvenance {
        self.module.parsed.provenance()
    }

    pub fn matches_prepared_declarations(
        &self,
        declarations: &ast_segmenter::PreparedDeclarations,
    ) -> bool {
        declarations.fingerprint() == self.module().facts().ecco_declarations_hash()
    }

    pub fn matches_declarations(&self, declarations: &ecco::LocalFacts) -> bool {
        ast_segmenter::owned::declarations_hash(declarations)
            .is_ok_and(|hash| hash == self.module().facts().ecco_declarations_hash())
    }

    pub fn from_parsed(source: &str, parsed: &ParsedModule) -> eyre::Result<Self> {
        crate::sidecar::enforce_policies(std::path::Path::new("<source>"), source, parsed.facts())?;
        Self::from_parsed_reporting_policies(source, parsed)
    }

    /// Preserve policy evidence without admitting it; callers enforce current policy.
    pub fn from_parsed_reporting_policies(
        source: &str,
        parsed: &ParsedModule,
    ) -> eyre::Result<Self> {
        ensure!(
            source.len() <= u32::MAX as usize,
            "cache source exceeds span limit"
        );
        let encoded = {
            let _span = tracing::info_span!("cache.encode").entered();
            parsed.to_flatbuffer(source, OXC_REVISION)
        }
        .and_then(|bytes| Self::decode_owned(bytes, source, parsed.provenance().config()));
        match encoded {
            Ok(record) => Ok(record),
            Err(error) => {
                // Wire limits must not turn a valid extraction into a fatal
                // source error. Only checked owned extraction may stay uncached;
                // malformed persistent input can never enter this fallback.
                crate::validate_parsed(source, parsed)?;
                parsed.owned_facts()?;
                Ok(Self {
                    wire: None,
                    module: Arc::new(CachedModule {
                        parsed: parsed.clone(),
                        segments: OnceLock::new(),
                    }),
                    source_len: source.len() as u64,
                    source_hash: digest(source.as_bytes()),
                    write_error: Some(format!("{error:#}").into()),
                })
            }
        }
    }

    fn check_source(&self, source: &str) -> eyre::Result<()> {
        ensure!(
            self.source_len() == source.len() as u64
                && self.source_hash() == digest(source.as_bytes()),
            "cache source mismatch"
        );
        Ok(())
    }

    /// Reuse the already semantically checked immutable owner, not an owned tree.
    pub fn to_parsed(&self, source: &str) -> eyre::Result<ParsedModule> {
        let parsed = self.to_parsed_reporting_policies(source)?;
        crate::sidecar::enforce_policies(
            std::path::Path::new("<cached source>"),
            source,
            parsed.facts(),
        )?;
        Ok(parsed)
    }

    /// Borrow the checked owner, retaining evidence for explicit caller admission.
    pub fn to_parsed_reporting_policies(&self, source: &str) -> eyre::Result<ParsedModule> {
        let _span = tracing::info_span!("cache.to_parsed").entered();
        self.check_source(source)?;
        Ok(self.module.parsed.clone())
    }

    pub fn encode(&self, source: &str) -> eyre::Result<Vec<u8>> {
        let _span = tracing::info_span!("cache.encode").entered();
        self.check_source(source)?;
        match &self.wire {
            Some(wire) => Ok(wire.bytes().to_vec()),
            None => eyre::bail!(
                "{}",
                self.write_error.as_deref().unwrap_or("uncacheable record")
            ),
        }
    }

    /// Invalid/stale records are errors here; filesystem callers treat them as misses.
    pub fn decode(bytes: &[u8], source: &str, config: ParserConfig) -> eyre::Result<Self> {
        let record = Self::decode_reporting_policies(bytes, source, config)?;
        crate::sidecar::enforce_policies(
            std::path::Path::new("<cached source>"),
            source,
            record.module().facts(),
        )?;
        Ok(record)
    }

    /// Decode checked borrowed evidence without applying caller admission.
    pub fn decode_reporting_policies(
        bytes: &[u8],
        source: &str,
        config: ParserConfig,
    ) -> eyre::Result<Self> {
        ensure!(bytes.len() <= MAX_RECORD_BYTES, "cache exceeds byte limit");
        Self::decode_shared(Arc::from(bytes), source, config)
    }

    pub(crate) fn decode_owned(
        bytes: Vec<u8>,
        source: &str,
        config: ParserConfig,
    ) -> eyre::Result<Self> {
        Self::decode_shared(bytes.into(), source, config)
    }

    fn decode_shared(bytes: Arc<[u8]>, source: &str, config: ParserConfig) -> eyre::Result<Self> {
        let _span = tracing::info_span!("cache.decode", bytes = bytes.len()).entered();
        let wire = {
            let _span = tracing::info_span!("cache.verify_flatbuffer").entered();
            cache_flatbuffers::WireRecord::new(bytes, source, config.wire(), OXC_REVISION)?
        };
        let parsed = ParsedModule::from_wire(source, wire.clone())?;
        let module = CachedModule {
            parsed,
            segments: OnceLock::new(),
        };
        // Integrity was checked above. This gate traverses borrowed records once,
        // before any CacheRecord can escape, including on the cold write path.
        crate::validate::validate_module(
            &(&module).into(),
            source,
            module.parsed.provenance().config(),
        )?;
        Ok(Self {
            source_len: wire.root().source_len(),
            source_hash: std::array::from_fn(|i| wire.root().source_hash().bytes().get(i)),
            wire: Some(wire),
            module: Arc::new(module),
            write_error: None,
        })
    }

    pub fn decode_with_declarations(
        bytes: &[u8],
        source: &str,
        config: ParserConfig,
        declarations: &ecco::LocalFacts,
    ) -> eyre::Result<Self> {
        let declarations = ast_segmenter::PreparedDeclarations::new(declarations.clone())?;
        Self::decode_with_prepared_declarations(bytes, source, config, &declarations)
    }

    pub fn decode_with_prepared_declarations(
        bytes: &[u8],
        source: &str,
        config: ParserConfig,
        declarations: &ast_segmenter::PreparedDeclarations,
    ) -> eyre::Result<Self> {
        let record = Self::decode(bytes, source, config)?;
        record.check_declarations(declarations)?;
        Ok(record)
    }

    pub(crate) fn check_declarations(
        &self,
        declarations: &ast_segmenter::PreparedDeclarations,
    ) -> eyre::Result<()> {
        ensure!(
            self.matches_prepared_declarations(declarations),
            "cache Ecco declaration configuration mismatch"
        );
        Ok(())
    }
}
