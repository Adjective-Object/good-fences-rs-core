use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueUse {
    Discarded,
    /// A variable initializer/default export; demand for its statement keeps the result.
    StatementValue,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifierCallee {
    name: String,
    binding: Option<BindingId>,
}

impl IdentifierCallee {
    pub(crate) fn new(name: String, binding: Option<BindingId>) -> Self {
        Self { name, binding }
    }
}

impl IdentifierCallee {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn binding(&self) -> Option<BindingId> {
        self.binding
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalleeShape {
    Identifier(IdentifierCallee),
    Member {
        #[serde(with = "crate::owned::span_serde")]
        receiver: Span,
        receiver_identifier: Option<IdentifierCallee>,
        /// Static name or computed string literal, not an inferred runtime key.
        property: Option<String>,
        computed: bool,
    },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationArgument {
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
    /// True requires iteration, not just evaluation of the iterable expression.
    spread: bool,
}

impl InvocationArgument {
    pub(crate) fn new(span: Span, spread: bool) -> Self {
        Self { span, spread }
    }
}

impl InvocationArgument {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn spread(&self) -> bool {
        self.spread
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
    #[serde(with = "crate::owned::span_serde")]
    callee: Span,
    shape: CalleeShape,
    arguments: Vec<InvocationArgument>,
    construct: bool,
    optional_call: bool,
    /// The callee's original chain can short-circuit before argument evaluation.
    optional_chain: bool,
    #[serde(with = "crate::owned::span_serde::optional")]
    pure: Option<Span>,
    #[serde(with = "crate::owned::span_serde::optional")]
    deferred: Option<Span>,
    value_use: ValueUse,
}

impl Invocation {
    pub(crate) fn new(
        span: Span,
        callee: Span,
        shape: CalleeShape,
        arguments: Vec<InvocationArgument>,
        construct: bool,
        optional_call: bool,
        optional_chain: bool,
        pure: Option<Span>,
        deferred: Option<Span>,
        value_use: ValueUse,
    ) -> Self {
        Self {
            span,
            callee,
            shape,
            arguments,
            construct,
            optional_call,
            optional_chain,
            pure,
            deferred,
            value_use,
        }
    }
}

impl Invocation {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn callee(&self) -> Span {
        self.callee
    }

    pub fn shape(&self) -> &CalleeShape {
        &self.shape
    }

    pub fn arguments(&self) -> &[InvocationArgument] {
        &self.arguments
    }

    pub fn construct(&self) -> bool {
        self.construct
    }

    pub fn optional_call(&self) -> bool {
        self.optional_call
    }

    pub fn optional_chain(&self) -> bool {
        self.optional_chain
    }

    pub fn pure(&self) -> Option<Span> {
        self.pure
    }

    pub fn deferred(&self) -> Option<Span> {
        self.deferred
    }

    pub fn value_use(&self) -> ValueUse {
        self.value_use
    }
}
