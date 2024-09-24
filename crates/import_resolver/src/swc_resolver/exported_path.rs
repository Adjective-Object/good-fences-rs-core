use serde::{Deserialize, Deserializer};

// Either a json string or a boolean
#[derive(Debug, Clone)]
pub enum ExportedPath {
    Exported(String),
    Private,
    // fallback option, see https://github.com/serde-rs/serde/issues/2057#issuecomment-879440712
    //
    // Some packages use non-standard extensions to the "exports" field
    // that are not supported.
    //
    // Rather than completely failing to parse the exports field, we ignore the exported
    // paths here.
    Unrecognized,
}

impl ExportedPath {
    pub fn as_ref(&'_ self) -> ExportedPathRef<'_> {
        match self {
            ExportedPath::Exported(s) => ExportedPathRef::Exported(s),
            ExportedPath::Private => ExportedPathRef::Private,
            ExportedPath::Unrecognized => ExportedPathRef::Unrecognized,
        }
    }

    pub fn map_export(&self, f: impl FnOnce(&str) -> String) -> Self {
        match self {
            ExportedPath::Exported(s) => ExportedPath::Exported(f(s)),
            ExportedPath::Private => ExportedPath::Private,
            ExportedPath::Unrecognized => ExportedPath::Unrecognized,
        }
    }
}

impl Default for ExportedPath {
    fn default() -> Self {
        Self::Unrecognized
    }
}

impl From<Option<String>> for ExportedPath {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(v) => ExportedPath::Exported(v),
            None => ExportedPath::Private,
        }
    }
}

impl PartialEq for ExportedPath {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExportedPath::Exported(a), ExportedPath::Exported(b)) => a == b,
            (ExportedPath::Private, ExportedPath::Private) => true,
            (ExportedPath::Unrecognized, ExportedPath::Unrecognized) => true,
            _ => false,
        }
    }
}

// Either a json string or a boolean
#[derive(Debug, Clone)]
pub enum ExportedPathRef<'a> {
    Exported(&'a str),
    Private,
    // fallback option, see https://github.com/serde-rs/serde/issues/2057#issuecomment-879440712
    //
    // Some packages use non-standard extensions to the "exports" field
    // that are not supported.
    //
    // Rather than completely failing to parse the exports field, we ignore the exported
    // paths here.
    Unrecognized,
}

impl<'a> ExportedPathRef<'a> {
    pub fn map_export<'b>(self, f: impl FnOnce(&'a str) -> &'b str) -> ExportedPathRef<'b> {
        match self {
            ExportedPathRef::Exported(s) => ExportedPathRef::Exported(f(s)),
            ExportedPathRef::Private => ExportedPathRef::Private,
            ExportedPathRef::Unrecognized => ExportedPathRef::Unrecognized,
        }
    }
}

impl<'a> Default for ExportedPathRef<'a> {
    fn default() -> Self {
        Self::Unrecognized
    }
}

impl<'a> From<Option<&'a str>> for ExportedPathRef<'a> {
    fn from(value: Option<&'a str>) -> Self {
        match value {
            Some(v) => ExportedPathRef::Exported(v),
            None => ExportedPathRef::Private,
        }
    }
}

impl<'a> PartialEq for ExportedPathRef<'a> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExportedPathRef::Exported(a), ExportedPathRef::Exported(b)) => a == b,
            (ExportedPathRef::Private, ExportedPathRef::Private) => true,
            _ => false,
        }
    }
}

struct ExportedPathVisitor;

impl<'de> serde::de::Visitor<'de> for ExportedPathVisitor {
    type Value = ExportedPath;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Private)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Private)
    }

    fn visit_string<E>(self, s: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Exported(s))
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Exported(String::from(s)))
    }

    fn visit_borrowed_str<E>(self, s: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Exported(String::from(s)))
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if !v {
            Ok(ExportedPath::Private)
        } else {
            Ok(ExportedPath::Unrecognized)
        }
    }

    fn visit_i8<E>(self, _: i8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_i16<E>(self, _: i16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_i32<E>(self, _: i32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_i128<E>(self, _: i128) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_u8<E>(self, _: u8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_u16<E>(self, _: u16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_u32<E>(self, _: u32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_u128<E>(self, _: u128) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_f32<E>(self, _: f32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_char<E>(self, _: char) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_borrowed_bytes<E>(self, _: &'de [u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_byte_buf<E>(self, _: Vec<u8>) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_newtype_struct<D>(self, _: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_seq<A>(self, _seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_map<A>(self, _map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        Ok(ExportedPath::Unrecognized)
    }

    fn visit_enum<A>(self, _data: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::EnumAccess<'de>,
    {
        Ok(ExportedPath::Unrecognized)
    }
}

impl<'de> Deserialize<'de> for ExportedPath {
    fn deserialize<D>(deserializer: D) -> Result<ExportedPath, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ExportedPathVisitor)
    }
}

#[cfg(test)]
mod test {
    use super::ExportedPath;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_deserialize_notexported() {
        let p: ExportedPath = serde_json::from_str("null").unwrap();
        assert_eq!(p, ExportedPath::Private);
    }

    #[test]
    fn test_deserialize_exportedpath() {
        let p: ExportedPath = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(p, ExportedPath::Exported("abc".to_string()));
    }

    #[test]
    fn test_deserialize_unrecognized() {
        let p: ExportedPath = serde_json::from_str("{}").unwrap();
        assert_eq!(p, ExportedPath::Unrecognized);
    }
}
