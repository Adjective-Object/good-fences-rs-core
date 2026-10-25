use ahashmap::AHashMap;
use ftree_cache::context_data::ContextData;
use serde::Deserialize;

use crate::exported_path::ExportedPath;

// Either a json string or a boolean
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum StringOrBool {
    Str(String),
    Bool(bool),
}

// package.json .browser field
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Browser {
    Str(String),
    Obj(BrowserMap),
}

pub type BrowserMap = AHashMap<String, StringOrBool>;

// Subset of package.json used during file resolution
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PackageJson {
    pub name: Option<String>,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub browser: Option<Browser>,
    #[serde(default, deserialize_with = "deserialize_exports")]
    pub exports: Option<PackageJsonExports>,
}

pub type PackageJsonExports = AHashMap<String, PackageJsonExport>;

fn deserialize_exports<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PackageJsonExports>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Exports {
        Root(String),
        Fallback(Vec<ExportedPath>),
        Conditions(
            #[serde(deserialize_with = "deserialize_root_conditions")] Vec<(String, ExportedPath)>,
        ),
        Subpaths(PackageJsonExports),
    }
    Ok(
        Option::<Exports>::deserialize(deserializer)?.map(|exports| match exports {
            Exports::Root(path) => [
                (".".to_owned(), PackageJsonExport::Single(Some(path))),
                (
                    "./*".to_owned(),
                    PackageJsonExport::Single(Some("./*".to_owned())),
                ),
            ]
            .into_iter()
            .collect(),
            Exports::Conditions(conditions) => {
                [(".".to_owned(), PackageJsonExport::Conditional(conditions))]
                    .into_iter()
                    .collect()
            }
            Exports::Fallback(targets) => [(".".to_owned(), PackageJsonExport::Fallback(targets))]
                .into_iter()
                .collect(),
            Exports::Subpaths(paths) => paths,
        }),
    )
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum PackageJsonExport {
    // An un-nested hashmap of that only maps the index of the module to the path
    //
    // e.g:
    // {
    //   "import": "./module.js",
    //   "require": "./main.js"
    //   "default": "./main.js"
    // }
    Single(Option<String>),
    Fallback(Vec<ExportedPath>),
    // An export condition object, kept in declaration order:
    //
    // e.g:
    // {
    //   ".": {
    //     "import": "./module.js",
    //     "require": "./main.js"
    //     "default": "./main.js"
    //   },
    //   "./lib/util": {
    //     "import": "./lib/util.esm",
    //     "require": "./lib/util.cjs"
    //     "default": "./lib/util.js"
    //   }
    // }
    Conditional(#[serde(deserialize_with = "deserialize_conditions")] Vec<(String, ExportedPath)>),
}

pub(crate) fn deserialize_conditions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, ExportedPath)>, D::Error> {
    deserialize_condition_map::<D, false>(deserializer)
}

fn deserialize_root_conditions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, ExportedPath)>, D::Error> {
    deserialize_condition_map::<D, true>(deserializer)
}

fn deserialize_condition_map<'de, D: serde::Deserializer<'de>, const ROOT: bool>(
    deserializer: D,
) -> Result<Vec<(String, ExportedPath)>, D::Error> {
    struct Conditions<const ROOT: bool>;
    impl<'de, const ROOT: bool> serde::de::Visitor<'de> for Conditions<ROOT> {
        type Value = Vec<(String, ExportedPath)>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an ordered export condition object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut conditions: Vec<(String, ExportedPath)> = Vec::new();
            while let Some(condition) = map.next_key::<String>()? {
                if ROOT && condition.starts_with('.') {
                    return Err(serde::de::Error::custom("subpath map, not root conditions"));
                }
                let target = map.next_value::<ExportedPath>()?;
                if let Some((_, previous)) =
                    conditions.iter_mut().find(|(key, _)| key == &condition)
                {
                    *previous = target;
                } else {
                    conditions.push((condition, target));
                }
            }
            Ok(conditions)
        }
    }
    deserializer.deserialize_map(Conditions::<ROOT>)
}

impl ContextData for PackageJson {
    fn read_context_data(
        _: (),
        path: &std::path::Path,
    ) -> anyhow::Result<Option<Self>, anyhow::Error> {
        let bytes = match std::fs::read(path) {
            Ok(f) => f,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }

                return Err(e.into());
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.into())
    }

    fn read_context_data_with_inputs(
        _: (),
        path: &std::path::Path,
        observer: &ftree_cache::inputs::InputObserver,
    ) -> anyhow::Result<Option<Self>> {
        observer
            .read(path)
            .map_err(|error| anyhow::Error::from_boxed(error.into()))?
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_conditions_normalize_to_an_ordered_exact_root_export() {
        let shorthand: PackageJson = serde_json::from_str(
            r#"{"exports":{"default":{"import":{"types":"./types.d.ts","default":"./index.js"}},"custom":"./custom.js"}}"#,
        ).unwrap();
        let explicit: PackageJson = serde_json::from_str(
            r#"{"exports":{".":{"default":{"import":{"types":"./types.d.ts","default":"./index.js"}},"custom":"./custom.js"}}}"#,
        ).unwrap();
        assert_eq!(shorthand, explicit);
        assert_eq!(shorthand.exports.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn string_exports_normalize_to_root_and_package_file_access() {
        let shorthand: PackageJson = serde_json::from_str(r#"{"exports":"./index.mjs"}"#).unwrap();
        let explicit: PackageJson =
            serde_json::from_str(r#"{"exports":{".":"./index.mjs","./*":"./*"}}"#).unwrap();
        assert_eq!(shorthand, explicit);
        for invalid in ["false", "42"] {
            assert!(
                serde_json::from_str::<PackageJson>(&format!(r#"{{"exports":{invalid}}}"#))
                    .is_err()
            );
        }
    }

    #[test]
    fn root_fallback_arrays_normalize_without_adding_package_file_access() {
        for targets in [
            "[]",
            r#"["./first.js","./second.js"]"#,
            r#"[{"import":["./first.js",null]},"./fallback.js"]"#,
        ] {
            let shorthand: PackageJson =
                serde_json::from_str(&format!(r#"{{"exports":{targets}}}"#)).unwrap();
            let explicit: PackageJson =
                serde_json::from_str(&format!(r#"{{"exports":{{".":{targets}}}}}"#)).unwrap();
            assert_eq!(shorthand, explicit);
            assert_eq!(shorthand.exports.as_ref().unwrap().len(), 1);
        }
    }
}
