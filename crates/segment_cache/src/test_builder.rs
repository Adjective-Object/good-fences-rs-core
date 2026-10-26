//! Deliberately unvalidated records for corruption tests, never a production mutation API.
use crate::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// Owned fixture projection only. Production records never implement Serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FixtureModule {
    body: ModuleBody,
    #[serde(with = "ast_segmenter::owned::span_serde::list")]
    segments: Vec<Span>,
    facts: ast_segmenter::owned::ModuleFacts,
}

impl FixtureModule {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FixtureRecord {
    magic: String,
    schema: u32,
    extractor: u32,
    oxc: String,
    source_len: u64,
    source_hash: [u8; 16],
    config: ParserConfig,
    module: FixtureModule,
    provenance: ast_segmenter::SourceProvenance,
}

impl FixtureRecord {
    pub fn from_parsed(source: &str, parsed: &ParsedModule) -> eyre::Result<Self> {
        CacheRecord::from_parsed(source, parsed)?;
        Ok(Self {
            magic: "good-fences-segments".into(),
            schema: SCHEMA_VERSION,
            extractor: EXTRACTOR_VERSION,
            oxc: OXC_REVISION.into(),
            source_len: source.len() as u64,
            source_hash: digest(source.as_bytes()),
            config: parsed.provenance().config(),
            module: FixtureModule {
                body: parsed.body(),
                segments: parsed.segments().iter().map(|s| s.span).collect(),
                facts: parsed.owned_facts()?.clone(),
            },
            provenance: parsed.provenance().clone(),
        })
    }
    pub fn config(&self) -> ParserConfig {
        self.config
    }
    pub fn module(&self) -> &FixtureModule {
        &self.module
    }
    pub fn to_parsed(&self, source: &str) -> eyre::Result<ParsedModule> {
        CacheRecord::decode(&self.envelope(false), source, self.config)?.to_parsed(source)
    }
    pub fn encode(&self, source: &str) -> eyre::Result<Vec<u8>> {
        let bytes = self.envelope(false);
        CacheRecord::decode(&bytes, source, self.config)?;
        Ok(bytes)
    }
    pub fn envelope(&self, reseal: bool) -> Vec<u8> {
        use cache_flatbuffers::wire as w;
        let module = self
            .module
            .facts
            .encode_module(self.module.body, &self.module.segments);
        let p = serde_json::to_value(&self.provenance).unwrap();
        let source_hash: [u8; 16] = serde_json::from_value(p["source_hash"].clone()).unwrap();
        let records_hash: [u8; 16] = if reseal {
            digest(&module)
        } else {
            serde_json::from_value(p["records_hash"].clone()).unwrap()
        };
        let config = self.config.wire();
        let provenance_config = self.provenance.config().wire();
        let hash = w::Hash128::new(&self.source_hash);
        let p_hash = w::Hash128::new(&source_hash);
        let records_hash = w::Hash128::new(&records_hash);
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let module = b.create_vector(&module);
        let oxc = b.create_string(&self.oxc);
        let provenance = w::SourceProvenance::create(
            &mut b,
            &w::SourceProvenanceArgs {
                source_hash: Some(&p_hash),
                config: Some(&provenance_config),
                records_hash: Some(&records_hash),
            },
        );
        let root = w::CacheRecord::create(
            &mut b,
            &w::CacheRecordArgs {
                schema: self.schema,
                extractor: self.extractor,
                oxc: Some(oxc),
                source_len: self.source_len,
                source_hash: Some(&hash),
                config: Some(&config),
                module: Some(module),
                provenance: Some(provenance),
            },
        );
        w::finish_size_prefixed_cache_record_buffer(&mut b, root);
        // Deliberately bypass writer validation to exercise the reader on real
        // malformed wire records, including unsupported discriminants/versions.
        let mut bytes = b.finished_data().to_vec();
        if self.magic != "good-fences-segments" {
            bytes[8..12].fill(0);
        }
        bytes.extend_from_slice(&digest(&bytes));
        bytes
    }
}

impl PartialEq<CacheRecord> for FixtureRecord {
    fn eq(&self, other: &CacheRecord) -> bool {
        self.envelope(false) == other.wire_bytes_for_test()
    }
}

pub(super) fn corrupt<T: Serialize + DeserializeOwned>(
    original: &T,
    edit: impl FnOnce(&mut Value),
) -> T {
    let mut value = serde_json::to_value(original).unwrap();
    edit(&mut value);
    serde_json::from_value(value).unwrap()
}

pub(super) trait RecordBuilder {
    fn at(&mut self, path: &str) -> &mut Value;
    fn set(&mut self, path: &str, value: impl Serialize) {
        *self.at(path) = serde_json::to_value(value).unwrap();
    }
    fn copy(&mut self, target: &str, source: &str) {
        let value = self.at(source).clone();
        self.set(target, value);
    }
    fn clear(&mut self, path: &str) {
        match self.at(path) {
            Value::Array(values) => values.clear(),
            Value::String(value) => value.clear(),
            _ => panic!("not a collection: {path}"),
        }
    }
    fn pop(&mut self, path: &str) {
        self.at(path).as_array_mut().unwrap().pop();
    }
    fn push(&mut self, path: &str, value: impl Serialize) {
        self.at(path)
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(value).unwrap());
    }
    fn add(&mut self, path: &str, amount: u64) {
        let value = self.at(path).as_u64().unwrap() + amount;
        self.set(path, value);
    }
    fn reverse(&mut self, path: &str) {
        self.at(path).as_array_mut().unwrap().reverse();
    }
}

impl RecordBuilder for Value {
    fn at(&mut self, path: &str) -> &mut Value {
        let mut current = self;
        for part in path.split('.') {
            current = if current.is_array() {
                let index = match part {
                    "start" => 0,
                    "end" => 1,
                    _ => part.parse().unwrap(),
                };
                &mut current[index]
            } else {
                current
                    .get_mut(part)
                    .unwrap_or_else(|| panic!("missing field {part}"))
            };
        }
        current
    }
}
