//! Predefined loader policy, independent of build configuration and pipeline dispatch.
//!
//! Labels select syntax, not module format. Original input identity, package format
//! and explicit m/c extension semantics must be preserved separately by consumers.
use std::{collections::BTreeMap, fmt, path::Path, str::FromStr};

use eyre::{ensure, OptionExt};
use serde::{de::MapAccess, Deserialize, Deserializer, Serialize};

pub(crate) mod file;
pub(crate) mod json;
pub(crate) mod locstring;
pub(crate) mod script;
pub(crate) mod stylesheet;

/// A predefined pipeline, never a callback or plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Loader {
    Js,
    Jsx,
    Ts,
    Tsx,
    Json,
    Jsonc,
    Css,
    CssModules,
    Scss,
    ScssModules,
    Sass,
    SassModules,
    Locstring,
    File,
}

impl FromStr for Loader {
    type Err = eyre::Report;

    fn from_str(label: &str) -> eyre::Result<Self> {
        Ok(match label {
            "js" => Self::Js,
            "jsx" => Self::Jsx,
            "ts" => Self::Ts,
            "tsx" => Self::Tsx,
            "json" => Self::Json,
            "jsonc" => Self::Jsonc,
            "css" => Self::Css,
            "css-modules" => Self::CssModules,
            "scss" => Self::Scss,
            "scss-modules" => Self::ScssModules,
            "sass" => Self::Sass,
            "sass-modules" => Self::SassModules,
            "locstring" => Self::Locstring,
            "file" => Self::File,
            _ => eyre::bail!("unknown loader label {label:?}"),
        })
    }
}

impl<'de> Deserialize<'de> for Loader {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Script language only; does not decide ESM versus CommonJS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScriptSyntax {
    Js,
    Jsx,
    Ts,
    Tsx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StylesheetSyntax {
    Css,
    Scss,
    Sass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StylesheetScoping {
    Global,
    Modules,
}

impl Loader {
    pub fn script_syntax(self) -> Option<ScriptSyntax> {
        match self {
            Self::Js => Some(ScriptSyntax::Js),
            Self::Jsx => Some(ScriptSyntax::Jsx),
            Self::Ts => Some(ScriptSyntax::Ts),
            Self::Tsx => Some(ScriptSyntax::Tsx),
            _ => None,
        }
    }

    /// Syntax and scoping come from the selected label, even for custom suffixes.
    pub fn stylesheet(self) -> Option<(StylesheetSyntax, StylesheetScoping)> {
        use StylesheetScoping::{Global, Modules};
        use StylesheetSyntax::{Css, Sass, Scss};
        match self {
            Self::Css => Some((Css, Global)),
            Self::CssModules => Some((Css, Modules)),
            Self::Scss => Some((Scss, Global)),
            Self::ScssModules => Some((Scss, Modules)),
            Self::Sass => Some((Sass, Global)),
            Self::SassModules => Some((Sass, Modules)),
            _ => None,
        }
    }
}

/// Effective, canonical policy after overlaying defaults per suffix.
///
/// Serialization exposes the effective map in alphabetical order; deserialization
/// accepts a strict map of suffixes to labels or null and overlays the defaults.
/// Null is an explicit rejection, not removal of a rule. No filesystem IO occurs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoaderPolicy {
    rules: BTreeMap<String, Option<Loader>>,
}

impl Default for LoaderPolicy {
    fn default() -> Self {
        use Loader::*;
        Self {
            rules: [
                (".js", Js),
                (".mjs", Js),
                (".cjs", Js),
                (".jsx", Jsx),
                (".mjsx", Jsx),
                (".cjsx", Jsx),
                (".ts", Ts),
                (".mts", Ts),
                (".cts", Ts),
                (".tsx", Tsx),
                (".mtsx", Tsx),
                (".ctsx", Tsx),
                (".json", Json),
                (".jsonc", Jsonc),
                (".css", Css),
                (".global.css", Css),
                (".module.css", CssModules),
                (".scss", Scss),
                (".global.scss", Scss),
                (".module.scss", ScssModules),
                (".sass", Sass),
                (".global.sass", Sass),
                (".module.sass", SassModules),
                (".locstring.json", Locstring),
                ("*", File),
            ]
            .into_iter()
            .map(|(suffix, loader)| (suffix.to_owned(), Some(loader)))
            .collect(),
        }
    }
}

impl LoaderPolicy {
    /// Overlay defaults. Duplicate keys are errors, never order-dependent wins.
    pub fn from_overrides<K: AsRef<str>>(
        overrides: impl IntoIterator<Item = (K, Option<Loader>)>,
    ) -> eyre::Result<Self> {
        let mut rules = BTreeMap::new();
        for (suffix, loader) in overrides {
            let suffix = suffix.as_ref();
            validate_suffix(suffix)?;
            ensure!(
                rules.insert(suffix.to_owned(), loader).is_none(),
                "duplicate loader suffix {suffix:?}"
            );
        }
        let mut policy = Self::default();
        policy.rules.extend(rules);
        Ok(policy)
    }

    /// Replace one effective rule, consuming the old policy. Null rejects.
    pub fn with_override(mut self, suffix: &str, loader: Option<Loader>) -> eyre::Result<Self> {
        validate_suffix(suffix)?;
        self.rules.insert(suffix.to_owned(), loader);
        Ok(self)
    }

    /// Borrow the effective map for cache keys and inspection, without mutation.
    pub fn rules(&self) -> &BTreeMap<String, Option<Loader>> {
        &self.rules
    }

    /// Match only the filename, case-sensitively; longest suffix wins.
    ///
    /// A matching null blocks both shorter suffixes and the wildcard. Declaration
    /// files and native addons are rejected before any configured mapping.
    pub fn select(&self, path: &Path) -> eyre::Result<Loader> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_eyre("loader selection requires a UTF-8 filename")?;
        ensure!(
            ![".d.ts", ".d.mts", ".d.cts"]
                .iter()
                .any(|suffix| name.ends_with(suffix)),
            "TypeScript declaration files are not executable inputs: {}",
            path.display()
        );
        ensure!(
            !name.ends_with(".node"),
            "native addons are not supported inputs: {}",
            path.display()
        );
        let (suffix, loader) = name
            .match_indices('.')
            .find_map(|(offset, _)| self.rules.get_key_value(&name[offset..]))
            .or_else(|| self.rules.get_key_value("*"))
            .ok_or_eyre("loader policy has no fallback")?;
        loader
            .ok_or_else(|| eyre::eyre!("loader suffix {suffix:?} rejects input {}", path.display()))
    }

    /// CSS import context supplies CSS only when no concrete suffix rule matches.
    /// Explicit file/script/JSON rules and null remain authoritative.
    pub(crate) fn select_stylesheet(&self, path: &Path) -> eyre::Result<Loader> {
        let selected = self.select(path)?;
        if selected.stylesheet().is_some() {
            return Ok(selected);
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        ensure!(
            !name
                .match_indices('.')
                .any(|(offset, _)| self.rules.contains_key(&name[offset..])),
            "stylesheet input has incompatible loader {selected:?}: {}",
            path.display()
        );
        Ok(Loader::Css)
    }

    /// Enabled resolver extensions, without leading dots, shortest first then
    /// alphabetical. Compound suffixes remain intact; null and `*` are excluded.
    /// This does not implement resolver probing or exact-file precedence.
    pub fn resolution_extensions(&self) -> Vec<&str> {
        let mut suffixes: Vec<_> = self
            .rules
            .iter()
            .filter(|(_, loader)| loader.is_some())
            .filter_map(|(suffix, _)| suffix.strip_prefix('.'))
            .collect();
        suffixes.sort_unstable_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        suffixes
    }
}

fn validate_suffix(suffix: &str) -> eyre::Result<()> {
    ensure!(
        suffix == "*"
            || (suffix.starts_with('.')
                && suffix[1..].split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                })),
        "invalid loader suffix {suffix:?}: expected * or a dotted suffix such as .module.css"
    );
    Ok(())
}

impl<'de> Deserialize<'de> for LoaderPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PolicyVisitor;

        impl<'de> serde::de::Visitor<'de> for PolicyVisitor {
            type Value = LoaderPolicy;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map of filename suffixes to loader labels or null")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut rules = BTreeMap::new();
                while let Some(suffix) = map.next_key::<String>()? {
                    if rules.contains_key(&suffix) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate loader suffix {suffix:?}"
                        )));
                    }
                    rules.insert(suffix, map.next_value::<Option<Loader>>()?);
                }
                LoaderPolicy::from_overrides(rules).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_map(PolicyVisitor)
    }
}

#[cfg(test)]
mod tests;
