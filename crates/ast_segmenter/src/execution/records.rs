use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionFacts {
    /// Preorder contexts; `None` denotes module evaluation.
    contexts: Vec<ExecutionContext>,
    /// Same indices as the canonical reference and invocation collections.
    references: Vec<Option<u32>>,
    invocations: Vec<Option<u32>>,
}

impl ExecutionFacts {
    pub fn contexts(&self) -> &[ExecutionContext] {
        &self.contexts
    }

    pub fn references(&self) -> &[Option<u32>] {
        &self.references
    }

    pub fn invocations(&self) -> &[Option<u32>] {
        &self.invocations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextKind {
    Callable,
    InstanceInitializer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
    parent: Option<u32>,
    kind: ContextKind,
    /// Only a synchronous, non-generator literal callee is proven immediate.
    /// Named/aliased calls and callback invocation remain conservative.
    immediate: Option<u32>,
}

impl ExecutionContext {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn parent(&self) -> Option<u32> {
        self.parent
    }

    pub fn kind(&self) -> ContextKind {
        self.kind
    }

    pub fn immediate(&self) -> Option<u32> {
        self.immediate
    }
}

impl ExecutionFacts {
    pub(super) fn contexts_mut(&mut self) -> &mut Vec<ExecutionContext> {
        &mut self.contexts
    }
    pub(super) fn references_mut(&mut self) -> &mut Vec<Option<u32>> {
        &mut self.references
    }
    pub(super) fn invocations_mut(&mut self) -> &mut Vec<Option<u32>> {
        &mut self.invocations
    }
}

impl ExecutionContext {
    pub(super) fn new(
        span: Span,
        parent: Option<u32>,
        kind: ContextKind,
        immediate: Option<u32>,
    ) -> Self {
        Self {
            span,
            parent,
            kind,
            immediate,
        }
    }
}
