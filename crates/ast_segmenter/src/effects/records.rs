use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectReason {
    UnknownExpression,
    UnknownControl,
    Invocation,
    Construction,
    PropertyRead,
    Coercion,
    Mutation,
    SpreadOrIteration,
    Destructuring,
    ClassHeritage,
    ComputedKey,
    StaticBlock,
    UnresolvedRead,
    BindingRead,
    OpaqueCommonJs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectSite {
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
    reason: EffectReason,
    /// None means module evaluation; otherwise execution waits for this function
    /// or instance field initializer. Lexical containment is not a call graph.
    #[serde(with = "crate::owned::span_serde::optional")]
    deferred: Option<Span>,
}

impl EffectSite {
    pub(crate) fn new(span: Span, reason: EffectReason, deferred: Option<Span>) -> Self {
        Self {
            span,
            reason,
            deferred,
        }
    }
}

impl EffectSite {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn reason(&self) -> EffectReason {
        self.reason
    }

    pub fn deferred(&self) -> Option<Span> {
        self.deferred
    }
}
