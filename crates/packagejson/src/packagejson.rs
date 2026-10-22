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

fn deserialize_conditions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, ExportedPath)>, D::Error> {
    struct Conditions;
    impl<'de> serde::de::Visitor<'de> for Conditions {
        type Value = Vec<(String, ExportedPath)>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an ordered export condition object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut conditions: Vec<(String, ExportedPath)> = Vec::new();
            while let Some((condition, target)) = map.next_entry::<String, ExportedPath>()? {
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
    deserializer.deserialize_map(Conditions)
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
        use ftree_cache::inputs::PathInput;
        let mut input = match PathInput::capture(path) {
            Ok(input) => input,
            Err(error) => {
                observer.error(error);
                return Self::read_context_data((), path);
            }
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = input.validate() {
                    observer.error(error);
                }
                observer.record(input);
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        match PathInput::capture(path) {
            Ok(after) if after == input => {}
            Ok(_) => observer.error(format!(
                "manifest identity changed while reading {}",
                path.display()
            )),
            Err(error) => observer.error(error),
        }
        input.seal_content(&bytes);
        observer.record(input);
        serde_json::from_slice(&bytes).map(Some).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_exports_normalize_to_root_and_package_file_access() {
        let shorthand: PackageJson = serde_json::from_str(r#"{"exports":"./index.mjs"}"#).unwrap();
        let explicit: PackageJson =
            serde_json::from_str(r#"{"exports":{".":"./index.mjs","./*":"./*"}}"#).unwrap();
        assert_eq!(shorthand, explicit);
        for invalid in ["false", "42", "[]"] {
            assert!(
                serde_json::from_str::<PackageJson>(&format!(r#"{{"exports":{invalid}}}"#))
                    .is_err()
            );
        }
    }
}
