use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmissionFacts {
    execution: crate::execution::ExecutionFacts,
    /// Same indices as `ParsedModule::segments`; CommonJS has one opaque recipe.
    segments: Vec<SegmentRecipe>,
    nested_bindings: Vec<BindingId>,
    free_references: Vec<u32>,
}

impl EmissionFacts {
    pub(crate) fn new(
        execution: crate::execution::ExecutionFacts,
        segments: Vec<SegmentRecipe>,
        nested_bindings: Vec<BindingId>,
        free_references: Vec<u32>,
    ) -> Self {
        Self {
            execution,
            segments,
            nested_bindings,
            free_references,
        }
    }
}

impl EmissionFacts {
    pub fn execution(&self) -> &crate::execution::ExecutionFacts {
        &self.execution
    }

    pub fn segments(&self) -> &[SegmentRecipe] {
        &self.segments
    }

    pub fn nested_bindings(&self) -> &[BindingId] {
        &self.nested_bindings
    }

    pub fn free_references(&self) -> &[u32] {
        &self.free_references
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentRecipe {
    /// Original-source assembly pieces, independent of the segment's ownership range.
    source: crate::MultiSpan,
    connector_only: bool,
    placement: PlacementFacts,
    names: NameRestrictions,
    namespaces: Vec<NamespaceUse>,
    /// Preorder, with parents before children. A replacing parent owns its children.
    operations: Vec<Operation>,
}

impl SegmentRecipe {
    pub(crate) fn new(
        source: crate::MultiSpan,
        connector_only: bool,
        placement: PlacementFacts,
        names: NameRestrictions,
        namespaces: Vec<NamespaceUse>,
        operations: Vec<Operation>,
    ) -> Self {
        Self {
            source,
            connector_only,
            placement,
            names,
            namespaces,
            operations,
        }
    }
}

impl SegmentRecipe {
    pub fn source(&self) -> &crate::MultiSpan {
        &self.source
    }

    pub fn connector_only(&self) -> bool {
        self.connector_only
    }

    pub fn placement(&self) -> &PlacementFacts {
        &self.placement
    }

    pub fn names(&self) -> &NameRestrictions {
        &self.names
    }

    pub fn namespaces(&self) -> &[NamespaceUse] {
        &self.namespaces
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementFacts {
    declarations: Vec<BindingId>,
    /// Top-level function declarations available during module instantiation.
    hoisted: Vec<BindingId>,
    /// All declarations in this segment are hoisted function declarations.
    instantiable: bool,
    default_expression: bool,
    forward_references: Vec<BindingId>,
    /// Indices into `ModuleFacts::references`, not OXC reference IDs.
    eager_reads: Vec<u32>,
    eager_writes: Vec<u32>,
    writes: Vec<u32>,
}

impl PlacementFacts {
    pub fn declarations(&self) -> &[BindingId] {
        &self.declarations
    }

    pub fn hoisted(&self) -> &[BindingId] {
        &self.hoisted
    }

    pub fn instantiable(&self) -> bool {
        self.instantiable
    }

    pub fn default_expression(&self) -> bool {
        self.default_expression
    }

    pub fn forward_references(&self) -> &[BindingId] {
        &self.forward_references
    }

    pub fn eager_reads(&self) -> &[u32] {
        &self.eager_reads
    }

    pub fn eager_writes(&self) -> &[u32] {
        &self.eager_writes
    }

    pub fn writes(&self) -> &[u32] {
        &self.writes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameRestrictions {
    /// Renaming one of these function/class names changes observable `.name`.
    named: Vec<BindingId>,
    /// Preserve the existing conservative anonymous function/class/arrow gate.
    inference_sensitive: bool,
}

impl NameRestrictions {
    pub fn named(&self) -> &[BindingId] {
        &self.named
    }

    pub fn inference_sensitive(&self) -> bool {
        self.inference_sensitive
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shorthand {
    #[default]
    None,
    Property,
    /// Object literals need a computed key to avoid the __proto__ setter.
    Proto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceUse {
    /// Any import-binding use; only linked Namespace identities select this alternative.
    reference: u32,
    access: NamespaceAccess,
}

impl NamespaceUse {
    pub(crate) fn new(reference: u32, access: NamespaceAccess) -> Self {
        Self { reference, access }
    }
}

impl NamespaceUse {
    pub fn reference(&self) -> u32 {
        self.reference
    }

    pub fn access(&self) -> &NamespaceAccess {
        &self.access
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespaceAccess {
    DirectRead {
        #[serde(with = "crate::owned::span_serde")]
        member: Span,
        property: String,
    },
    Native(NamespaceReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespaceReason {
    Other,
    Receiver,
    Tag,
    Write,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    parent: Option<u32>,
    kind: OperationKind,
}

impl Operation {
    pub(crate) fn new(parent: Option<u32>, kind: OperationKind) -> Self {
        Self { parent, kind }
    }
}

impl Operation {
    pub fn parent(&self) -> Option<u32> {
        self.parent
    }

    pub fn kind(&self) -> &OperationKind {
        &self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    Binding {
        binding: BindingId,
        declaration: u32,
        shorthand: Shorthand,
    },
    Reference {
        reference: u32,
        shorthand: Shorthand,
    },
    Namespace {
        usage: u32,
    },
    Dependency {
        dependency: DependencyId,
    },
    Export {
        /// Export-list facts address individual specifiers, not this wrapper.
        #[serde(with = "crate::owned::span_serde")]
        span: Span,
        form: ExportForm,
    },
    SourceHint {
        #[serde(with = "crate::owned::span_serde")]
        span: Span,
    },
    Invocation {
        invocation: u32,
        lowering: PureLowering,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportForm {
    Remove,
    Declaration {
        #[serde(with = "crate::owned::span_serde")]
        declaration: Span,
    },
    /// Consumers may keep the native default wrapper instead of using this alternative.
    Default {
        value: SourceSlot,
        hoisted: bool,
    },
}

/// Original source plus already-composed operation roots for this subexpression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSlot {
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
    operations: Vec<u32>,
}

impl SourceSlot {
    pub(crate) fn new(span: Span, operations: Vec<u32>) -> Self {
        Self { span, operations }
    }
}

impl SourceSlot {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn operations(&self) -> &[u32] {
        &self.operations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PureLowering {
    callee: SourceSlot,
    /// `None` rejects nested optional calls/private chains, as the current backend does.
    chain: Option<CalleePlan>,
    arguments: Vec<SourceSlot>,
    /// First original guard boundary; parentheses terminate the outer chain.
    guard_start: Option<u32>,
    crosses_await_guard: bool,
}

impl PureLowering {
    pub(crate) fn new(
        callee: SourceSlot,
        chain: Option<CalleePlan>,
        arguments: Vec<SourceSlot>,
        guard_start: Option<u32>,
        crosses_await_guard: bool,
    ) -> Self {
        Self {
            callee,
            chain,
            arguments,
            guard_start,
            crosses_await_guard,
        }
    }
}

impl PureLowering {
    pub fn callee(&self) -> &SourceSlot {
        &self.callee
    }

    pub fn chain(&self) -> Option<&CalleePlan> {
        self.chain.as_ref()
    }

    pub fn arguments(&self) -> &[SourceSlot] {
        &self.arguments
    }

    pub fn guard_start(&self) -> Option<u32> {
        self.guard_start
    }

    pub fn crosses_await_guard(&self) -> bool {
        self.crosses_await_guard
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalleePlan {
    base: SourceSlot,
    /// Inner to outer, so receiver evaluation and call receiver identity survive.
    steps: Vec<ChainStep>,
}

impl CalleePlan {
    pub(crate) fn new(base: SourceSlot, steps: Vec<ChainStep>) -> Self {
        Self { base, steps }
    }
}

impl CalleePlan {
    pub fn base(&self) -> &SourceSlot {
        &self.base
    }

    pub fn steps(&self) -> &[ChainStep] {
        &self.steps
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainStep {
    Static {
        property: String,
        optional: bool,
    },
    Computed {
        property: SourceSlot,
        optional: bool,
    },
    Call {
        /// Includes parentheses, arguments and intervening trivia.
        suffix: SourceSlot,
    },
}

impl EmissionFacts {
    pub(super) fn segments_mut(&mut self) -> &mut Vec<SegmentRecipe> {
        &mut self.segments
    }
}

impl SegmentRecipe {
    pub(super) fn operations_mut(&mut self) -> &mut Vec<Operation> {
        &mut self.operations
    }
    pub(super) fn placement_mut(&mut self) -> &mut PlacementFacts {
        &mut self.placement
    }
    pub(super) fn names_mut(&mut self) -> &mut NameRestrictions {
        &mut self.names
    }
    pub(super) fn namespaces_mut(&mut self) -> &mut Vec<NamespaceUse> {
        &mut self.namespaces
    }
    pub(super) fn connector_only_mut(&mut self) -> &mut bool {
        &mut self.connector_only
    }
}

impl PlacementFacts {
    pub(super) fn declarations_mut(&mut self) -> &mut Vec<BindingId> {
        &mut self.declarations
    }
    pub(super) fn hoisted_mut(&mut self) -> &mut Vec<BindingId> {
        &mut self.hoisted
    }
    pub(super) fn forward_references_mut(&mut self) -> &mut Vec<BindingId> {
        &mut self.forward_references
    }
    pub(super) fn eager_reads_mut(&mut self) -> &mut Vec<u32> {
        &mut self.eager_reads
    }
    pub(super) fn eager_writes_mut(&mut self) -> &mut Vec<u32> {
        &mut self.eager_writes
    }
    pub(super) fn writes_mut(&mut self) -> &mut Vec<u32> {
        &mut self.writes
    }
    pub(super) fn instantiable_mut(&mut self) -> &mut bool {
        &mut self.instantiable
    }
    pub(super) fn default_expression_mut(&mut self) -> &mut bool {
        &mut self.default_expression
    }
}

impl NameRestrictions {
    pub(super) fn named_mut(&mut self) -> &mut Vec<BindingId> {
        &mut self.named
    }
    pub(super) fn inference_sensitive_mut(&mut self) -> &mut bool {
        &mut self.inference_sensitive
    }
}

impl Operation {
    pub(super) fn parent_mut(&mut self) -> &mut Option<u32> {
        &mut self.parent
    }
    pub(super) fn kind_mut(&mut self) -> &mut OperationKind {
        &mut self.kind
    }
}

impl SourceSlot {
    pub(super) fn operations_mut(&mut self) -> &mut Vec<u32> {
        &mut self.operations
    }
}

impl PureLowering {
    pub(super) fn crosses_await_guard_mut(&mut self) -> &mut bool {
        &mut self.crosses_await_guard
    }
    pub(super) fn callee_mut(&mut self) -> &mut SourceSlot {
        &mut self.callee
    }
    pub(super) fn arguments_mut(&mut self) -> &mut Vec<SourceSlot> {
        &mut self.arguments
    }
    pub(super) fn chain_mut(&mut self) -> &mut Option<CalleePlan> {
        &mut self.chain
    }
}

impl CalleePlan {
    pub(super) fn base_mut(&mut self) -> &mut SourceSlot {
        &mut self.base
    }
    pub(super) fn steps_mut(&mut self) -> &mut Vec<ChainStep> {
        &mut self.steps
    }
}
