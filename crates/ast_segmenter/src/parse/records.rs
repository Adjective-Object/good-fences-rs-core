use super::*;

/// Immutable owned-or-wire parse facts, not transformed JavaScript or an executable bundle.
#[derive(Debug, Clone)]
pub struct ParsedModule {
    source_type: SourceType,
    body: ModuleBody,
    segments: Option<Vec<Segment>>,
    facts: crate::views::FactsStorage,
    provenance: crate::SourceProvenance,
}

impl ParsedModule {
    pub fn into_facts(self) -> crate::views::FactsStorage {
        self.facts
    }

    pub fn into_parts(
        self,
    ) -> (
        SourceType,
        ModuleBody,
        Vec<Segment>,
        crate::views::FactsStorage,
        crate::SourceProvenance,
    ) {
        let facts = &self.facts;
        let segments = self.segments.unwrap_or_else(|| {
            let crate::views::FactsStorage::Wire(wire) = facts else {
                unreachable!("owned extraction retains its segments")
            };
            wire.module()
                .segments()
                .iter()
                .map(|s| Segment {
                    span: Span::new(s.start(), s.end()),
                    module_deps: Default::default(),
                    variables: Default::default(),
                })
                .collect()
        });
        (
            self.source_type,
            self.body,
            segments,
            self.facts,
            self.provenance,
        )
    }

    pub fn source_type(&self) -> SourceType {
        self.source_type
    }

    pub fn body(&self) -> ModuleBody {
        self.body
    }

    pub fn segments(&self) -> crate::views::Sequence<'_, crate::views::SegmentRange> {
        match &self.segments {
            Some(segments) => crate::views::Sequence::Owned(segments),
            None => {
                let crate::views::FactsStorage::Wire(wire) = &self.facts else {
                    unreachable!("owned extraction retains its segments")
                };
                crate::views::Sequence::Wire(wire.module().segments())
            }
        }
    }

    pub fn owned_segments(&self) -> Option<&[Segment]> {
        self.segments.as_deref()
    }

    pub fn facts(&self) -> crate::views::ModuleFacts<'_> {
        self.facts.view()
    }

    pub fn owned_facts(&self) -> eyre::Result<&crate::owned::ModuleFacts> {
        self.facts.owned()
    }
}

impl ParsedModule {
    pub fn provenance(&self) -> &crate::SourceProvenance {
        &self.provenance
    }

    /// Reconstitute owned extraction records only with matching extraction/cache provenance.
    pub fn from_provenance(
        source: &str,
        provenance: crate::SourceProvenance,
        body: ModuleBody,
        segments: Vec<Segment>,
        facts: impl Into<crate::views::FactsStorage>,
    ) -> eyre::Result<Self> {
        let facts = facts.into();
        provenance.verify(
            source,
            provenance.config(),
            body,
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
            facts.view(),
        )?;
        Ok(Self {
            source_type: provenance.config().source_type(),
            body,
            segments: Some(segments),
            facts,
            provenance,
        })
    }
}

impl ParsedModule {
    pub(super) fn new(
        source_type: SourceType,
        body: ModuleBody,
        segments: Vec<Segment>,
        facts: crate::owned::ModuleFacts,
        provenance: crate::SourceProvenance,
    ) -> Self {
        Self {
            source_type,
            body,
            segments: Some(segments),
            facts: facts.into(),
            provenance,
        }
    }
}
impl ParsedModule {
    /// Retain a verified owner without unpacking syntax/Ecco trees. Callers loading
    /// persistent input must additionally perform source-local semantic validation.
    pub fn from_wire(source: &str, wire: cache_flatbuffers::WireRecord) -> eyre::Result<Self> {
        let provenance = crate::SourceProvenance::from_wire(source, &wire)?;
        let body = crate::views::module_body(wire.module());
        Ok(Self {
            source_type: provenance.config().source_type(),
            body,
            segments: None,
            facts: crate::views::FactsStorage::Wire(wire),
            provenance,
        })
    }
}
