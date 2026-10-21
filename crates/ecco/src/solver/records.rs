use super::{Effect, ValueId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallResult {
    value: ValueId,
    effects: Effect,
}

impl CallResult {
    pub(crate) fn new(value: ValueId, effects: Effect) -> Self {
        Self { value, effects }
    }

    pub fn value(&self) -> ValueId {
        self.value
    }

    pub fn effects(&self) -> Effect {
        self.effects
    }
}
