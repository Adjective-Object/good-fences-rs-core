use super::*;

/// Owned parse facts, not transformed JavaScript or an executable bundle.
#[derive(Debug, Clone)]
pub struct ParsedModule {
    source_type: SourceType,
    body: ModuleBody,
    segments: Vec<Segment>,
    facts: crate::owned::ModuleFacts,
    provenance: crate::SourceProvenance,
}

impl ParsedModule {
    pub fn into_facts(self) -> crate::owned::ModuleFacts {
        self.facts
    }

    pub fn into_parts(
        self,
    ) -> (
        SourceType,
        ModuleBody,
        Vec<Segment>,
        crate::owned::ModuleFacts,
        crate::SourceProvenance,
    ) {
        (
            self.source_type,
            self.body,
            self.segments,
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

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn facts(&self) -> &crate::owned::ModuleFacts {
        &self.facts
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
        facts: crate::owned::ModuleFacts,
    ) -> eyre::Result<Self> {
        provenance.verify(
            source,
            provenance.config(),
            body,
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
            &facts,
        )?;
        Ok(Self {
            source_type: provenance.config().source_type(),
            body,
            segments,
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
            segments,
            facts,
            provenance,
        }
    }
}
