//! Validate selection once, before loading any graph inputs.
use std::path::Path;

use eyre::ensure;

/// CSS Modules selection is independent of stylesheet loading.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub enum CssModules {
    #[default]
    Disabled,
    Enabled,
    Everything,
    Extension(String),
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum Value {
    Bool(bool),
    String(String),
}

impl TryFrom<Value> for CssModules {
    type Error = eyre::Report;

    fn try_from(value: Value) -> eyre::Result<Self> {
        let value = match value {
            Value::Bool(false) => Self::Disabled,
            Value::Bool(true) => Self::Enabled,
            Value::String(value) if value == "everything" => Self::Everything,
            Value::String(value) => Self::Extension(
                value
                    .strip_prefix("ext:")
                    .ok_or_else(|| {
                        eyre::eyre!(
                            "css_modules expects false, true, \"everything\" or \"ext:.suffix\""
                        )
                    })?
                    .to_owned(),
            ),
        };
        value.validate()?;
        Ok(value)
    }
}

impl From<CssModules> for Value {
    fn from(value: CssModules) -> Self {
        match value {
            CssModules::Disabled => Self::Bool(false),
            CssModules::Enabled => Self::Bool(true),
            CssModules::Everything => Self::String("everything".into()),
            CssModules::Extension(suffix) => Self::String(format!("ext:{suffix}")),
        }
    }
}

impl CssModules {
    pub(crate) fn validate(&self) -> eyre::Result<()> {
        if let Self::Extension(suffix) = self {
            validate_suffix(suffix, "css_modules")?;
        }
        Ok(())
    }

    pub(crate) fn matches(&self, path: &Path) -> bool {
        let suffix = match self {
            Self::Disabled => return false,
            Self::Everything => {
                return path
                    .extension()
                    .is_some_and(|ext| matches!(ext.to_str(), Some("css" | "scss" | "sass")));
            }
            Self::Enabled => {
                return path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        [".module.css", ".module.scss", ".module.sass"]
                            .iter()
                            .any(|suffix| name.ends_with(suffix))
                    });
            }
            Self::Extension(suffix) => suffix,
        };
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
    }
}

fn validate_suffix(suffix: &str, option: &str) -> eyre::Result<()> {
    ensure!(
        suffix.starts_with('.')
            && suffix[1..].split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            }),
        "invalid {option} extension {suffix:?}: expected a dotted suffix such as .module.css"
    );
    Ok(())
}

impl crate::build::CssOptions {
    pub(crate) fn validate(&self) -> eyre::Result<()> {
        self.css_modules.validate()?;
        for suffix in &self.css_modules_exclude {
            validate_suffix(suffix, "css_modules_exclude")?;
        }
        Ok(())
    }

    pub(crate) fn excludes_module(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                self.css_modules_exclude
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceKind;

    fn kind(path: &Path, selection: &CssModules) -> ResourceKind {
        crate::resource::kind(
            path,
            &crate::build::CssOptions {
                css_modules: selection.clone(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn selection_is_validated_and_round_trips() {
        for input in [
            "false",
            "true",
            r#""everything""#,
            r#""ext:.any.custom.extension""#,
        ] {
            let selection: CssModules = serde_json::from_str(input).unwrap();
            assert_eq!(serde_json::to_string(&selection).unwrap(), input);
        }
        for input in [
            r#""modules""#,
            r#""ext:""#,
            r#""ext:css""#,
            r#""ext:.""#,
            r#""ext:..css""#,
            r#""ext:.a..css""#,
            r#""ext:.css/""#,
            r#""ext:.css?x""#,
            r#""ext:.css x""#,
            "null",
            "3",
        ] {
            assert!(
                serde_json::from_str::<CssModules>(input).is_err(),
                "{input}"
            );
        }
        assert!(CssModules::Extension(String::new()).validate().is_err());
    }

    #[test]
    fn suffix_exclusions_override_selection_without_changing_resource_kinds() {
        let mut options = crate::build::CssOptions {
            css_modules: CssModules::Everything,
            css_modules_exclude: vec![
                ".global.css".into(),
                ".global.scss".into(),
                ".global.sass".into(),
            ],
            ..Default::default()
        };
        options.validate().unwrap();
        assert_eq!(
            serde_json::from_str::<crate::build::CssOptions>(
                &serde_json::to_string(&options).unwrap()
            )
            .unwrap(),
            options
        );
        for suffix in ["css", "scss", "sass"] {
            assert_eq!(
                crate::resource::kind(Path::new(&format!("theme.global.{suffix}")), &options),
                ResourceKind::Stylesheet { module: false }
            );
            assert_eq!(
                crate::resource::kind(Path::new(&format!("theme.module.{suffix}")), &options),
                ResourceKind::Stylesheet { module: true }
            );
        }
        assert_eq!(
            crate::resource::stylesheet_kind(Path::new("import.global.css"), &options),
            ResourceKind::Stylesheet { module: false }
        );
        assert_eq!(
            crate::resource::kind(Path::new("directory.global.css/local.css"), &options),
            ResourceKind::Stylesheet { module: true }
        );
        options.css_modules = CssModules::Extension(".js".into());
        options.css_modules_exclude = vec![".global.js".into()];
        assert_eq!(
            crate::resource::kind(Path::new("theme.global.js"), &options),
            ResourceKind::Stylesheet { module: false }
        );
        for suffix in ["", "global.css", "..css", ".css/path", ".css?raw"] {
            options.css_modules_exclude = vec![suffix.into()];
            let error = options.validate().unwrap_err();
            assert!(error.to_string().contains("css_modules_exclude"), "{error}");
        }
    }

    #[test]
    fn classification_precedes_assets_without_widening_everything() {
        let custom = CssModules::Extension(".any.custom.extension".into());
        assert_eq!(
            kind(Path::new("x.any.custom.extension"), &custom),
            ResourceKind::Stylesheet { module: true }
        );
        assert_eq!(
            kind(Path::new("x.module.css"), &CssModules::Enabled),
            ResourceKind::Stylesheet { module: true }
        );
        assert_eq!(
            kind(Path::new("x.module.css"), &CssModules::Disabled),
            ResourceKind::Stylesheet { module: false }
        );
        for suffix in ["scss", "sass"] {
            let path = format!("x.module.{suffix}");
            assert_eq!(
                kind(Path::new(&path), &CssModules::Disabled),
                ResourceKind::Stylesheet { module: false }
            );
            for selection in [CssModules::Enabled, CssModules::Everything] {
                assert_eq!(
                    kind(Path::new(&path), &selection),
                    ResourceKind::Stylesheet { module: true }
                );
            }
        }
        assert_eq!(
            kind(Path::new("x.css"), &custom),
            ResourceKind::Stylesheet { module: false }
        );
        for (path, expected) in [
            ("x.js", ResourceKind::Script),
            ("x.json", ResourceKind::Json),
            ("x.jsonc", ResourceKind::Json),
            ("x.bin", ResourceKind::Asset),
        ] {
            assert_eq!(kind(Path::new(path), &CssModules::Everything), expected);
        }
        assert_eq!(
            crate::resource::stylesheet_kind(
                Path::new("extensionless-import"),
                &crate::build::CssOptions {
                    css_modules: CssModules::Everything,
                    ..Default::default()
                }
            ),
            ResourceKind::Stylesheet { module: true }
        );
    }
}
