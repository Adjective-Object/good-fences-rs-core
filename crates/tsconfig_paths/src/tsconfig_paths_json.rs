use crate::error::OpenTsConfigError;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::vec::Vec;

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct RawTsconfigPathsJson {
    pub extends: Option<String>,
    pub compiler_options: Option<TsconfigPathsCompilerOptions>,
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct TsconfigPathsJson {
    pub compiler_options: TsconfigPathsCompilerOptions,
    pub exclude: Vec<String>,
}

impl TsconfigPathsJson {
    pub fn from_path(tsconfig_path: &str) -> Result<Self, OpenTsConfigError> {
        Self::resolve(Path::new(tsconfig_path))
    }

    fn resolve(path: &Path) -> Result<Self, OpenTsConfigError> {
        let file = File::open(path).map_err(OpenTsConfigError::IOError)?;
        let buf_reader = BufReader::new(file);
        let raw: RawTsconfigPathsJson =
            serde_json::from_reader(buf_reader).map_err(OpenTsConfigError::SerdeError)?;

        let base = match &raw.extends {
            Some(extends_path) => {
                let parent_dir = path.parent().unwrap_or(Path::new("."));
                let resolved = parent_dir.join(extends_path);
                Some(Self::resolve(&resolved)?)
            }
            None => None,
        };

        let mut compiler_options = base
            .as_ref()
            .map(|b| b.compiler_options.clone())
            .unwrap_or_default();

        if let Some(overrides) = raw.compiler_options {
            if overrides.base_url.is_some() {
                compiler_options.base_url = overrides.base_url;
            }
            if !overrides.paths.is_empty() {
                compiler_options.paths.extend(overrides.paths);
            }
        }

        // Child exclude overrides base exclude (same as tsc behavior)
        let exclude = match raw.exclude {
            Some(exc) => exc,
            None => base.map(|b| b.exclude).unwrap_or_default(),
        };

        Ok(TsconfigPathsJson { compiler_options, exclude })
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsCompilerOptions {
    pub base_url: Option<String>,
    #[serde(default)]
    pub paths: HashMap<String, Vec<String>>,
}
