use serde::Deserialize;
use std::path::PathBuf;

use swc_common::collections::{AHashMap, AHashSet};

use super::context_data::ContextData;

// Either a json string or a boolean
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum StringOrBool {
    Str(String),
    Bool(bool),
}

// package.json .browser field
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum Browser {
    Str(String),
    Obj(AHashMap<String, StringOrBool>),
}

// Subset of package.json used during file resolution
#[derive(Debug, Deserialize, Clone)]
pub struct PackageJson {
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub browser: Option<Browser>,
}

/// Processed data derived from the package.json file's .browser object field
#[derive(Debug, Default)]

struct BrowserRewriteCache {
    rewrites: AHashMap<PathBuf, PathBuf>,
    ignores: AHashSet<PathBuf>,
    module_rewrites: AHashMap<String, PathBuf>,
    module_ignores: AHashSet<String>,
}

impl ContextData for PackageJson {
    fn read_context_data(
        _: (),
        path: &std::path::Path,
    ) -> anyhow::Result<Option<Self>, anyhow::Error> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file)
            .map(Some)
            .map_err(|e| e.into())
    }
}
