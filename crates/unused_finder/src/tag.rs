use core::fmt::{self, Display};
use serde::{Deserialize, Serialize};

use crate::graph::UsedTag;

/// Bitflag used internally to represent the tags on a file or symbol
impl Display for UsedTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tags = Vec::new();
        if self.contains(Self::FROM_ENTRY) {
            tags.push("entry");
        };
        if self.contains(Self::FROM_IGNORED) {
            tags.push("ignored");
        };
        if self.contains(Self::FROM_TEST) {
            tags.push("test");
        };
        write!(f, "{}", tags.join("+"))
    }
}

/// External-facing enum type used to represent the tags on a file or symbol
#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedTagEnum {
    Entry,
    Ignored,
    Test,
}

impl From<UsedTag> for Option<Vec<UsedTagEnum>> {
    fn from(flags: UsedTag) -> Self {
        if flags.is_empty() {
            return None;
        }

        let result: Vec<UsedTagEnum> = flags.into();
        Some(result)
    }
}

impl From<UsedTag> for Vec<UsedTagEnum> {
    fn from(flags: UsedTag) -> Self {
        let mut result = Vec::new();
        if flags.contains(UsedTag::FROM_ENTRY) {
            result.push(UsedTagEnum::Entry);
        }
        if flags.contains(UsedTag::FROM_IGNORED) {
            result.push(UsedTagEnum::Ignored);
        }
        if flags.contains(UsedTag::FROM_TEST) {
            result.push(UsedTagEnum::Test);
        }

        result
    }
}
