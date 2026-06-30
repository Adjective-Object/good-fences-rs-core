use core::fmt::{self, Display};
use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct UsedTag: u8 {
        /// True if this file or symbol was used recursively by an
        /// "entry package" (a package that was passed as an entry point).
        const FROM_ENTRY = 0x01;
        /// True if this file or symbol was used recursively by a test file.
        const FROM_TEST = 0x02;
        /// True if this file or symbol was used recursively by an
        /// ignored symbol or file.
        const FROM_IGNORED = 0x04;
        // True if this symbol is a type-only symbol (interface, type alias)
        const TYPE_ONLY = 0x08;
        /// True if this symbol was reached through a non-type-only import edge
        const USED_AS_VALUE = 0x10;
        /// True if this symbol was reached through a type-only import edge
        const USED_AS_TYPE = 0x20;
    }
}

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
        }
        if self.contains(Self::TYPE_ONLY) {
            tags.push("type-only");
        }
        if self.contains(Self::USED_AS_VALUE) {
            tags.push("used-as-value");
        }
        if self.contains(Self::USED_AS_TYPE) {
            tags.push("used-as-type");
        }
        write!(f, "{}", tags.join("+"))
    }
}

#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedTagEnum {
    Entry,
    Ignored,
    Test,
    TypeOnly,
    UsedAsValue,
    UsedAsType,
}

impl Display for UsedTagEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsedTagEnum::Entry => write!(f, "entry"),
            UsedTagEnum::Ignored => write!(f, "ignored"),
            UsedTagEnum::Test => write!(f, "test"),
            UsedTagEnum::TypeOnly => write!(f, "type-only"),
            UsedTagEnum::UsedAsValue => write!(f, "used-as-value"),
            UsedTagEnum::UsedAsType => write!(f, "used-as-type"),
        }
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
        if flags.contains(UsedTag::TYPE_ONLY) {
            result.push(UsedTagEnum::TypeOnly);
        }
        if flags.contains(UsedTag::USED_AS_VALUE) {
            result.push(UsedTagEnum::UsedAsValue);
        }
        if flags.contains(UsedTag::USED_AS_TYPE) {
            result.push(UsedTagEnum::UsedAsType);
        }

        result
    }
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
