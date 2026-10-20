/// Flat operation hazards; membership hazards describe the right operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Hazard {
    PropertyRead,
    ToPrimitive,
    In,
    Instanceof,
    Call,
    Construct,
    Iterate,
}

impl Hazard {
    pub const ALL: [Self; 7] = [
        Self::PropertyRead,
        Self::ToPrimitive,
        Self::In,
        Self::Instanceof,
        Self::Call,
        Self::Construct,
        Self::Iterate,
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
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hazards(pub [bool; 7]);

impl Hazards {
    pub const UNKNOWN: Self = Self([true; 7]);
    pub const NONE: Self = Self([false; 7]);

    pub fn get(self, slot: Hazard) -> bool {
        self.0[slot as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Binding {
    Global(Vec<String>),
    Module { module: String, path: Vec<String> },
    Local(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HazardExpr {
    Bool(bool),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallArgument {
    Value(String),
    Spread(String),
    Forward,
}

/// Explicit fields override the wildcard default, independent of source order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Returns {
    #[default]
    Fresh,
    Argument(String),
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub target: String,
    /// None means all hazard slots.
    pub slot: Option<Hazard>,
    pub expression: HazardExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalFacts {
    pub definitions: Vec<Definition>,
}
