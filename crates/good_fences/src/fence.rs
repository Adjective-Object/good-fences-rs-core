use anyhow::{Context, Error, Result};
use relative_path::RelativePath;
use serde::de::{Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::path::Path;
use std::str::FromStr;
use void::Void;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Fence {
    // relative path to the fence, from the root of the workspace
    pub fence_path: String,
    // the parsed fence
    pub fence: ParsedFence,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ParsedFence {
    pub tags: Option<Vec<String>>,
    pub exports: Option<Vec<ExportRule>>,
    pub dependencies: Option<Vec<DependencyRule>>,
    pub imports: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RawDependencyRule {
    dependency: String,
    #[serde(default, deserialize_with = "expand_to_string_vec")]
    accessible_to: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Serialize)]
pub struct DependencyRule {
    pub dependency: String,
    pub accessible_to: Vec<String>,
}

impl From<RawDependencyRule> for DependencyRule {
    fn from(val: RawDependencyRule) -> Self {
        DependencyRule {
            dependency: val.dependency,
            accessible_to: match val.accessible_to {
                Some(a) => a,
                None => vec!["*".to_owned()],
            },
        }
    }
}

impl FromStr for DependencyRule {
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(DependencyRule {
            dependency: s.to_owned(),
            accessible_to: vec!["*".to_owned()],
        })
    }
}

impl<'de> Deserialize<'de> for DependencyRule {
    fn deserialize<D>(deserializer: D) -> Result<DependencyRule, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StringOrStructVisitor::<DependencyRule, RawDependencyRule>(
            PhantomData,
            PhantomData,
        ))
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RawExportRule {
    modules: String,
    #[serde(default, deserialize_with = "expand_to_string_vec")]
    accessible_to: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Serialize)]
pub struct ExportRule {
    pub accessible_to: Vec<String>,
    pub modules: String,
}

impl From<RawExportRule> for ExportRule {
    fn from(val: RawExportRule) -> Self {
        ExportRule {
            modules: val.modules,
            accessible_to: match val.accessible_to {
                Some(a) => a,
                None => vec!["*".to_owned()],
            },
        }
    }
}
impl FromStr for ExportRule {
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ExportRule {
            modules: s.to_owned(),
            accessible_to: vec!["*".to_owned()],
        })
    }
}

impl<'de> Deserialize<'de> for ExportRule {
    fn deserialize<D>(deserializer: D) -> Result<ExportRule, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StringOrStructVisitor::<ExportRule, RawExportRule>(
            PhantomData,
            PhantomData,
        ))
    }
}

struct StringOrStringArrayVisitor {}
impl<'de> Visitor<'de> for StringOrStringArrayVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "a string or string array")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(vec![s.to_owned()])
    }

    fn visit_seq<S>(self, mut seq_access: S) -> Result<Self::Value, S::Error>
    where
        S: serde::de::SeqAccess<'de>,
    {
        let mut aggregated_strings = match seq_access.size_hint() {
            Some(size) => Vec::with_capacity(size),
            None => Vec::new(),
        };

        while let Ok(Some(elem)) = seq_access.next_element() {
            aggregated_strings.push(elem);
        }

        Ok(aggregated_strings)
    }
}

fn expand_to_string_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    return match deserializer.deserialize_any(StringOrStringArrayVisitor {}) {
        Ok(x) => Ok(Some(x)),
        Err(e) => Err(e),
    };
}

// This is a Visitor that forwards string types to T's `FromStr` impl and
// forwards map types to T's `Deserialize` impl. The `PhantomData` is to
// keep the compiler from complaining about T being an unused generic type
// parameter. We need T in order to know the Value type for the Visitor
// impl.
struct StringOrStructVisitor<TOuter, TInner>(
    PhantomData<fn() -> TOuter>,
    PhantomData<fn() -> TInner>,
);
impl<'de, TInner, TOuter> Visitor<'de> for StringOrStructVisitor<TOuter, TInner>
where
    TOuter: Deserialize<'de> + FromStr<Err = Void>,
    TInner: Deserialize<'de> + Into<TOuter>,
{
    type Value = TOuter;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string or map")
    }

    fn visit_str<E>(self, value: &str) -> Result<TOuter, E>
    where
        E: serde::de::Error,
    {
        Ok(FromStr::from_str(value).unwrap())
    }

    fn visit_map<M>(self, map: M) -> Result<TOuter, M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        // `MapAccessDeserializer` is a wrapper that turns a `MapAccess`
        // into a `Deserializer`, allowing it to be used as the input to T's
        // `Deserialize` implementation. T then deserializes itself using
        // the entries from the map visitor.
        let inner_struct_result: Result<TInner, M::Error> =
            Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map));
        match inner_struct_result {
            Ok(x) => Ok(x.into()),
            Err(e) => Err(e),
        }
    }
}

pub fn parse_fence_str(fence_str: &str, fence_path: &RelativePath) -> Result<Fence, Error> {
    let fence = serde_json::from_str(fence_str)
        .with_context(|| format!("failed to parse fence from {:?}", fence_path,))?;

    Result::Ok(Fence {
        fence_path: fence_path.to_string(),
        fence,
    })
}

pub fn parse_fence_file<P: AsRef<RelativePath>>(fence_path: P) -> Result<Fence, Error> {
    let fence_path_ref = fence_path.as_ref();
    let fence_text = std::fs::read_to_string(fence_path_ref.to_path(Path::new(".")))
        .with_context(|| format!("error reading fence file \"{:?}\"", fence_path_ref))?;

    parse_fence_str(&fence_text, fence_path_ref)
}

impl Fence {
    pub fn path_relative_to(self: &mut Fence, base_path: &Path) {
        println!("relative! {:?}, {:?}", self.fence_path, base_path);
        self.fence_path = pathdiff::diff_paths(self.fence_path.clone(), base_path)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
    }
}

#[cfg(test)]
mod test {
    use crate::fence::{parse_fence_str, DependencyRule, ExportRule, Fence, ParsedFence};
    use anyhow::{Error, Result};
    use relative_path::RelativePath;
    use std::option::Option;

    #[test]
    fn loads_empty_fence() {
        let result: Result<Fence> = parse_fence_str(
            r#"
      {}
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::None,
                    dependencies: Option::None,
                    imports: Option::None,
                }
            },
        );
    }

    #[test]
    fn loads_tags_only() {
        let result = parse_fence_str(
            r#"
      {
        "tags": ["a", "b", "c"]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::Some(vec!("a".to_owned(), "b".to_owned(), "c".to_owned())),
                    exports: Option::None,
                    dependencies: Option::None,
                    imports: Option::None,
                }
            },
        )
    }

    #[test]
    fn loads_single_export_rule_accessible_to_str() {
        let result = parse_fence_str(
            r#"
      {
        "exports": [
          {
            "modules": "some_module",
            "accessibleTo": "accessible_to_one_other"
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::Some(vec!(ExportRule {
                        modules: "some_module".to_owned(),
                        accessible_to: vec!("accessible_to_one_other".to_owned())
                    })),
                    dependencies: Option::None,
                    imports: Option::None,
                }
            },
        )
    }

    #[test]
    fn loads_single_export_rule_missing_accessible_to() {
        let result = parse_fence_str(
            r#"
      {
        "exports": [
          {
            "modules": "some_module"
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::Some(vec!(ExportRule {
                        modules: "some_module".to_owned(),
                        accessible_to: vec!("*".to_owned())
                    })),
                    dependencies: Option::None,
                    imports: Option::None,
                }
            },
        )
    }

    #[test]
    fn loads_single_export_rule_accessible_to_str_arr() {
        let result = parse_fence_str(
            r#"
      {
        "exports": [
          {
            "modules": "some_module",
            "accessibleTo": ["accessible_to_other_1", "accessible_to_other_2"]
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::Some(vec!(ExportRule {
                        modules: "some_module".to_owned(),
                        accessible_to: vec!(
                            "accessible_to_other_1".to_owned(),
                            "accessible_to_other_2".to_owned()
                        )
                    })),
                    dependencies: Option::None,
                    imports: Option::None,
                }
            }
        )
    }

    #[test]
    fn loads_single_export_rule_as_str() {
        let result = parse_fence_str(
            r#"
      {
        "exports": [          
            "string_exported_module"
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::Some(vec!(ExportRule {
                        modules: "string_exported_module".to_owned(),
                        accessible_to: vec!("*".to_owned())
                    })),
                    dependencies: Option::None,
                    imports: Option::None,
                }
            }
        )
    }

    #[test]
    fn loads_single_dependency_rule_missing_accessible_to() {
        let result = parse_fence_str(
            r#"
      {
        "dependencies": [
          {
            "dependency": "some_dependency"
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::None,
                    dependencies: Option::Some(vec!(DependencyRule {
                        dependency: "some_dependency".to_owned(),
                        accessible_to: vec!("*".to_owned())
                    })),
                    imports: Option::None,
                }
            }
        )
    }

    #[test]
    fn loads_single_dependency_rule_accessible_to_str() {
        let result = parse_fence_str(
            r#"
      {
        "dependencies": [
          {
            "dependency": "some_dependency",
            "accessibleTo": "accessible_to_single_str"
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::None,
                    dependencies: Option::Some(vec!(DependencyRule {
                        dependency: "some_dependency".to_owned(),
                        accessible_to: vec!("accessible_to_single_str".to_owned())
                    })),
                    imports: Option::None,
                }
            },
        )
    }

    #[test]
    fn loads_single_dependency_rule_accessible_to_str_arr() {
        let result = parse_fence_str(
            r#"
      {
        "dependencies": [
          {
            "dependency": "some_dependency",
            "accessibleTo": ["accessible_to_other_1", "accessible_to_other_2"]
          }
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::None,
                    dependencies: Option::Some(vec!(DependencyRule {
                        dependency: "some_dependency".to_owned(),
                        accessible_to: vec!(
                            "accessible_to_other_1".to_owned(),
                            "accessible_to_other_2".to_owned()
                        )
                    })),
                    imports: Option::None,
                }
            },
        )
    }

    #[test]
    fn loads_single_dependency_rule_as_str() {
        let result: Result<Fence, Error> = parse_fence_str(
            r#"
      {
        "dependencies": [          
            "string_approved_dependency"
        ]
      }
      "#,
            RelativePath::new("test/path/to/fence.json"),
        );
        assert_eq!(
            result.unwrap(),
            Fence {
                fence_path: String::from("test/path/to/fence.json"),
                fence: ParsedFence {
                    tags: Option::None,
                    exports: Option::None,
                    dependencies: Option::Some(vec!(DependencyRule {
                        dependency: "string_approved_dependency".to_owned(),
                        accessible_to: vec!("*".to_owned(),)
                    })),
                    imports: Option::None,
                }
            }
        )
    }
}
