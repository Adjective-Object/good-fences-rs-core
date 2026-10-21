use serde::{Deserialize, Serialize};

/// Flat operation hazards; membership hazards describe the right operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(usize)]
pub enum Hazard {
    PropertyRead,
    ToPrimitive,
    In,
    Instanceof,
    Call,
    Construct,
    Iterate,
    PrototypeRead,
}

impl Hazard {
    pub const ALL: [Self; 8] = [
        Self::PropertyRead,
        Self::ToPrimitive,
        Self::In,
        Self::Instanceof,
        Self::Call,
        Self::Construct,
        Self::Iterate,
        Self::PrototypeRead,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::PropertyRead => "property_read_effects",
            Self::ToPrimitive => "toprimitive_effects",
            Self::In => "in_effects",
            Self::Instanceof => "instanceof_effects",
            Self::Call => "call_effects",
            Self::Construct => "construct_effects",
            Self::Iterate => "iterate_effects",
            Self::PrototypeRead => "prototype_read_effects",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hazards(pub [bool; Hazard::ALL.len()]);

impl Hazards {
    pub const UNKNOWN: Self = Self([true; Hazard::ALL.len()]);
    pub const NONE: Self = Self([false; Hazard::ALL.len()]);

    pub fn get(self, slot: Hazard) -> bool {
        self.0[slot as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Binding {
    Global(Vec<String>),
    Module { module: String, path: Vec<String> },
    Local(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HazardExpr {
    Bool(bool),
    Unknown,
    Read {
        value: String,
        slot: Hazard,
    },
    Or(Vec<Self>),
    Call {
        function: String,
        arguments: Vec<CallArgument>,
        receiver: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallArgument {
    Value(String),
    Spread(String),
    Forward,
}

/// Explicit fields override the wildcard default, independent of source order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HazardFields {
    default: Option<HazardExpr>,
    fields: Vec<(Hazard, HazardExpr)>,
}

impl HazardFields {
    pub fn new(default: Option<HazardExpr>, fields: Vec<(Hazard, HazardExpr)>) -> Self {
        Self { default, fields }
    }

    pub(crate) fn default_mut(&mut self) -> &mut Option<HazardExpr> {
        &mut self.default
    }

    pub(crate) fn fields_mut(&mut self) -> &mut Vec<(Hazard, HazardExpr)> {
        &mut self.fields
    }
}

impl HazardFields {
    pub fn default_expression(&self) -> Option<&HazardExpr> {
        self.default.as_ref()
    }

    pub fn fields(&self) -> &[(Hazard, HazardExpr)] {
        &self.fields
    }
}

impl HazardFields {
    pub fn get(&self, slot: Hazard) -> HazardExpr {
        self.fields
            .iter()
            .find(|(field, _)| *field == slot)
            .map(|(_, expression)| expression)
            .or(self.default.as_ref())
            .cloned()
            .unwrap_or(HazardExpr::Bool(true))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Returns {
    #[default]
    Fresh,
    Argument(String),
    Void,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    target: String,
    /// None means all hazard slots.
    slot: Option<Hazard>,
    expression: HazardExpr,
}

impl Update {
    pub fn new(target: String, slot: Option<Hazard>, expression: HazardExpr) -> Self {
        Self {
            target,
            slot,
            expression,
        }
    }
}

impl Update {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn slot(&self) -> Option<Hazard> {
        self.slot
    }

    pub fn expression(&self) -> &HazardExpr {
        &self.expression
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    binding: Binding,
    parameters: Vec<String>,
    rest: Option<String>,
    variadic: bool,
    hazards: HazardFields,
    returns: Returns,
    return_hazards: HazardFields,
    updates: Vec<Update>,
}

impl Definition {
    pub fn new(
        binding: Binding,
        parameters: Vec<String>,
        rest: Option<String>,
        variadic: bool,
        hazards: HazardFields,
        returns: Returns,
        return_hazards: HazardFields,
        updates: Vec<Update>,
    ) -> Self {
        Self {
            binding,
            parameters,
            rest,
            variadic,
            hazards,
            returns,
            return_hazards,
            updates,
        }
    }

    pub(crate) fn binding_mut(&mut self) -> &mut Binding {
        &mut self.binding
    }

    pub(crate) fn parameters_mut(&mut self) -> &mut Vec<String> {
        &mut self.parameters
    }

    pub(crate) fn rest_mut(&mut self) -> &mut Option<String> {
        &mut self.rest
    }

    pub(crate) fn variadic_mut(&mut self) -> &mut bool {
        &mut self.variadic
    }

    pub(crate) fn hazards_mut(&mut self) -> &mut HazardFields {
        &mut self.hazards
    }

    pub(crate) fn returns_mut(&mut self) -> &mut Returns {
        &mut self.returns
    }

    pub(crate) fn return_hazards_mut(&mut self) -> &mut HazardFields {
        &mut self.return_hazards
    }

    pub(crate) fn updates_mut(&mut self) -> &mut Vec<Update> {
        &mut self.updates
    }
}

impl Definition {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    pub fn parameters(&self) -> &[String] {
        &self.parameters
    }

    pub fn rest(&self) -> Option<&str> {
        self.rest.as_deref()
    }

    pub fn variadic(&self) -> bool {
        self.variadic
    }

    pub fn hazards(&self) -> &HazardFields {
        &self.hazards
    }

    pub fn returns(&self) -> &Returns {
        &self.returns
    }

    pub fn return_hazards(&self) -> &HazardFields {
        &self.return_hazards
    }

    pub fn updates(&self) -> &[Update] {
        &self.updates
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFacts {
    definitions: Vec<Definition>,
    global_symbols: Vec<GlobalSymbol>,
}

impl LocalFacts {
    pub fn new(definitions: Vec<Definition>, global_symbols: Vec<GlobalSymbol>) -> Self {
        Self {
            definitions,
            global_symbols,
        }
    }

    pub(crate) fn definitions_mut(&mut self) -> &mut Vec<Definition> {
        &mut self.definitions
    }

    pub(crate) fn global_symbols_mut(&mut self) -> &mut Vec<GlobalSymbol> {
        &mut self.global_symbols
    }

    pub fn into_parts(self) -> (Vec<Definition>, Vec<GlobalSymbol>) {
        (self.definitions, self.global_symbols)
    }
}

impl LocalFacts {
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    pub fn global_symbols(&self) -> &[GlobalSymbol] {
        &self.global_symbols
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSymbol {
    name: String,
    read_effects: bool,
}

impl GlobalSymbol {
    pub fn new(name: String, read_effects: bool) -> Self {
        Self { name, read_effects }
    }
}

impl GlobalSymbol {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn read_effects(&self) -> bool {
        self.read_effects
    }
}
