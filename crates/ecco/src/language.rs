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
    pub default: Option<HazardExpr>,
    pub fields: Vec<(Hazard, HazardExpr)>,
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
    pub target: String,
    /// None means all hazard slots.
    pub slot: Option<Hazard>,
    pub expression: HazardExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub binding: Binding,
    pub parameters: Vec<String>,
    pub rest: Option<String>,
    pub variadic: bool,
    pub hazards: HazardFields,
    pub returns: Returns,
    pub return_hazards: HazardFields,
    pub updates: Vec<Update>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFacts {
    pub definitions: Vec<Definition>,
    pub global_symbols: Vec<GlobalSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSymbol {
    pub name: String,
    pub read_effects: bool,
}
