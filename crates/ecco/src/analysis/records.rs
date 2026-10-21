use super::*;

/// Source-local declaration start, independent of OXC's arena and symbol IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u32);

/// Index into a parsed file's roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RootId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSymbol {
    id: SymbolId,
    name: String,
    #[serde(with = "span")]
    span: Span,
    value: ValueId,
}

impl ParsedSymbol {
    pub(crate) fn new(id: SymbolId, name: String, span: Span, value: ValueId) -> Self {
        Self {
            id,
            name,
            span,
            value,
        }
    }
}

impl ParsedSymbol {
    pub fn id(&self) -> SymbolId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn value(&self) -> ValueId {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalReference {
    binding: Binding,
    #[serde(with = "span")]
    span: Span,
}

impl ExternalReference {
    pub(crate) fn new(binding: Binding, span: Span) -> Self {
        Self { binding, span }
    }
}

impl ExternalReference {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRoot {
    #[serde(with = "span")]
    span: Span,
    function: OwnedFunction,
}

impl ParsedRoot {
    pub(crate) fn new(span: Span, function: OwnedFunction) -> Self {
        Self { span, function }
    }
}

impl ParsedRoot {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn function(&self) -> &OwnedFunction {
        &self.function
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ParsedValue {
    hazards: Hazards,
    definition: Option<usize>,
    alias: Option<ValueId>,
}

impl ParsedValue {
    pub(crate) fn new(hazards: Hazards, definition: Option<usize>, alias: Option<ValueId>) -> Self {
        Self {
            hazards,
            definition,
            alias,
        }
    }
}

impl ParsedValue {
    pub(crate) fn hazards(&self) -> Hazards {
        self.hazards
    }

    pub(crate) fn definition(&self) -> Option<usize> {
        self.definition
    }

    pub(crate) fn alias(&self) -> Option<ValueId> {
        self.alias
    }
}

/// Cacheable extraction only: no evaluated hazards, specialized calls, resolved
/// cross-file links, or interpreter state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    version: u32,
    facts: LocalFacts,
    values: Vec<ParsedValue>,
    functions: Vec<OwnedFunction>,
    setup: OwnedFunction,
    symbols: Vec<ParsedSymbol>,
    roots: Vec<ParsedRoot>,
    externals: Vec<ExternalReference>,
    matches: Vec<DefinitionMatch>,
}

impl ParsedFile {
    pub(crate) fn new(
        version: u32,
        facts: LocalFacts,
        values: Vec<ParsedValue>,
        functions: Vec<OwnedFunction>,
        setup: OwnedFunction,
        symbols: Vec<ParsedSymbol>,
        roots: Vec<ParsedRoot>,
        externals: Vec<ExternalReference>,
        matches: Vec<DefinitionMatch>,
    ) -> Self {
        Self {
            version,
            facts,
            values,
            functions,
            setup,
            symbols,
            roots,
            externals,
            matches,
        }
    }
}

impl ParsedFile {
    pub(crate) fn version(&self) -> u32 {
        self.version
    }

    pub(crate) fn values(&self) -> &[ParsedValue] {
        &self.values
    }

    pub(crate) fn setup(&self) -> &OwnedFunction {
        &self.setup
    }
}

impl ParsedFile {
    /// Top-level definitions, addressed by declaration spans. Imports are
    /// external references; the caller resolves through reexports to definitions.
    pub fn symbols(&self) -> &[ParsedSymbol] {
        &self.symbols
    }

    pub fn roots(&self) -> &[ParsedRoot] {
        &self.roots
    }

    pub fn functions(&self) -> &[OwnedFunction] {
        &self.functions
    }

    pub fn external_references(&self) -> &[ExternalReference] {
        &self.externals
    }

    pub fn declarations(&self) -> &LocalFacts {
        &self.facts
    }

    pub fn definition_matches(&self) -> &[DefinitionMatch] {
        &self.matches
    }
}
