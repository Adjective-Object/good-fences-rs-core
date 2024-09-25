use ftree_cache::context_data::ContextData;
use serde::Deserialize;
use swc_common::collections::AHashMap;

use super::exported_path::ExportedPath;

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
    Obj(BrowserMap),
}

pub type BrowserMap = AHashMap<String, StringOrBool>;

// Subset of package.json used during file resolution
#[derive(Debug, Deserialize, Clone)]
pub struct PackageJson {
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub browser: Option<Browser>,
    #[serde(default)]
    pub exports: Option<PackageJsonExports>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum PackageJsonExports {
    // An un-nested hashmap of that only maps the index of the module to the path
    //
    // e.g:
    // {
    //   "import": "./module.js",
    //   "require": "./main.js"
    //   "default": "./main.js"
    // }
    Single(AHashMap<String, Option<String>>),
    // A nested hashmap that maps multiple import paths into the module:
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
    Multiple(AHashMap<String, AHashMap<String, ExportedPath>>),
}

impl ContextData for PackageJson {
    fn read_context_data(
        _: (),
        path: &std::path::Path,
    ) -> anyhow::Result<Option<Self>, anyhow::Error> {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(e.into());
            }
        };
        serde_json::from_reader(file)
            .map(Some)
            .map_err(|e| e.into())
    }
}
