use crate::error::OpenTsConfigError;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::vec::Vec;

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsJson {
    pub compiler_options: TsconfigPathsCompilerOptions,
}

impl TsconfigPathsJson {
    // Reads and parses the tsconfig.json at the provided path
    pub fn from_path(tsconfig_path: &str) -> Result<Self, OpenTsConfigError> {
        let file = match File::open(tsconfig_path) {
            Ok(f) => f,
            Err(err) => return Err(OpenTsConfigError::IOError(err)),
        };
        let buf_reader = BufReader::new(file);
        let tsconfig_paths_json: TsconfigPathsJson = match serde_json::from_reader(buf_reader) {
            Ok(tsconfig) => tsconfig,
            Err(e) => return Err(OpenTsConfigError::SerdeError(e)),
        };
        Ok(tsconfig_paths_json)
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsCompilerOptions {
    pub base_url: Option<String>,
    pub paths: HashMap<String, Vec<String>>,
}
