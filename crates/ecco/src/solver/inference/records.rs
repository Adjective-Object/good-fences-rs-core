use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FunctionSignature {
    captures: Vec<ValueId>,
    parameters: Vec<ValueId>,
    receiver: Option<ValueId>,
    self_binding: Option<ValueId>,
    constructible: bool,
    arguments: Option<ValueId>,
}

impl FunctionSignature {
    pub(crate) fn new(
        captures: Vec<ValueId>,
        parameters: Vec<ValueId>,
        receiver: Option<ValueId>,
        self_binding: Option<ValueId>,
        constructible: bool,
        arguments: Option<ValueId>,
    ) -> Self {
        Self {
            captures,
            parameters,
            receiver,
            self_binding,
            constructible,
            arguments,
        }
    }
}

impl FunctionSignature {
    pub(crate) fn captures(&self) -> &[ValueId] {
        &self.captures
    }

    pub(crate) fn parameters(&self) -> &[ValueId] {
        &self.parameters
    }

    pub(crate) fn receiver(&self) -> Option<ValueId> {
        self.receiver
    }

    pub(crate) fn self_binding(&self) -> Option<ValueId> {
        self.self_binding
    }

    pub(crate) fn constructible(&self) -> bool {
        self.constructible
    }

    pub(crate) fn arguments(&self) -> Option<ValueId> {
        self.arguments
    }
}

/// Owned, lazy operations. IDs are local to the containing `ParsedFile`, not
/// handles into an interpreter. Serializing this never saves analyzed calls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFunction {
    signature: FunctionSignature,
    instructions: Vec<Instruction>,
    returns: Vec<ValueId>,
    effects: Option<Effect>,
    sources: std::collections::BTreeMap<Effect, (ValueId, Hazard)>,
}

impl OwnedFunction {
    pub(crate) fn new(
        signature: FunctionSignature,
        instructions: Vec<Instruction>,
        returns: Vec<ValueId>,
        effects: Option<Effect>,
        sources: std::collections::BTreeMap<Effect, (ValueId, Hazard)>,
    ) -> Self {
        Self {
            signature,
            instructions,
            returns,
            effects,
            sources,
        }
    }

    pub(crate) fn signature(&self) -> &FunctionSignature {
        &self.signature
    }

    pub(crate) fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub(crate) fn returns(&self) -> &[ValueId] {
        &self.returns
    }

    pub(crate) fn effects(&self) -> Option<Effect> {
        self.effects
    }

    pub(crate) fn sources(&self) -> &std::collections::BTreeMap<Effect, (ValueId, Hazard)> {
        &self.sources
    }
}
