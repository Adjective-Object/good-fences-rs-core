use super::*;

/// Projection of ParsedModule that preserves executable occurrence facts and
/// statement ranges without serializing legacy tracker tags or name sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModule {
    body: ModuleBody,
    #[serde(with = "ast_segmenter::owned::span_serde::list")]
    segments: Vec<Span>,
    facts: ModuleFacts,
}

impl CachedModule {
    pub fn body(&self) -> ModuleBody {
        self.body
    }

    pub fn segments(&self) -> &[Span] {
        &self.segments
    }

    pub fn facts(&self) -> ast_segmenter::views::ModuleFacts<'_> {
        (&self.facts).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRecord {
    magic: String,
    schema: u32,
    extractor: u32,
    oxc: String,
    source_len: u64,
    source_hash: [u8; 32],
    config: ParserConfig,
    module: CachedModule,
    provenance: ast_segmenter::SourceProvenance,
}

impl CacheRecord {
    pub fn magic(&self) -> &str {
        &self.magic
    }

    pub fn schema(&self) -> u32 {
        self.schema
    }

    pub fn extractor(&self) -> u32 {
        self.extractor
    }

    pub fn oxc(&self) -> &str {
        &self.oxc
    }

    pub fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn source_hash(&self) -> [u8; 32] {
        self.source_hash
    }

    pub fn config(&self) -> ParserConfig {
        self.config
    }

    pub fn module(&self) -> &CachedModule {
        &self.module
    }

    pub fn provenance(&self) -> &ast_segmenter::SourceProvenance {
        &self.provenance
    }
}

impl CacheRecord {
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
        let record = Self {
            magic: "good-fences-segments".into(),
            schema: SCHEMA_VERSION,
            extractor: EXTRACTOR_VERSION,
            oxc: OXC_REVISION.into(),
            source_len: source.len() as u64,
            source_hash: digest(source.as_bytes()),
            config: ParserConfig::from_source_type(parsed.source_type()),
            module: CachedModule {
                body: parsed.body(),
                segments: parsed.segments().iter().map(|s| s.span).collect(),
                facts: parsed.owned_facts()?.clone(),
            },
            provenance: parsed.provenance().clone(),
        };
        record.validate(source, record.config())?;
        Ok(record)
    }

    /// Checked cache adapter shared by graph construction and emission.
    pub fn to_parsed(&self, source: &str) -> eyre::Result<ParsedModule> {
        let _span = tracing::info_span!("cache.to_parsed").entered();
        self.validate(source, self.config())?;
        ParsedModule::from_provenance(
            source,
            self.provenance().clone(),
            self.module().body(),
            self.module()
                .segments()
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
        let _span = tracing::info_span!("cache.encode").entered();
        self.validate(source, self.config())?;
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
        let _span = tracing::info_span!("cache.decode", bytes = bytes.len()).entered();
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
        ensure!(
            record.matches_prepared_declarations(declarations),
            "cache Ecco declaration configuration mismatch"
        );
        Ok(record)
    }
}
