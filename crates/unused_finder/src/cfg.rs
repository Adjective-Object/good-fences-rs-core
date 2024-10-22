use ahashmap::AHashSet;
use js_err::JsErr;
use serde::Deserialize;

/// A JSON serializable proxy for the UnusedFinderConfig struct
///
/// This struct is used to serialize the UnusedFinderConfig struct to JSON
/// with serde, or to recieve the config to JS via napi.
#[cfg_attr(feature = "napi", napi(object))]
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedFinderJSONConfig {
    // Trace exported symbols that are not imported anywhere in the project
    #[serde(default)]
    pub report_exported_symbols: bool,
    // Root paths to walk as source files
    #[serde(alias = "pathsToRead")]
    pub root_paths: Vec<String>,
    // Path to the root directory of the repository.
    #[serde(default)]
    pub repo_root: String,
    // Files under matching dirs won't be scanned during the file walk
    #[serde(default)]
    pub skip: Vec<String>,
    pub entry_packages: Vec<String>,
}

/// Configuration for the unused symbols finder
#[derive(Debug, Default, Clone)]
pub struct UnusedFinderConfig {
    /// If true, the finder should report exported symbols that are not used anywhere in the project
    pub report_exported_symbols: bool,

    /// Path to the root directory of the repository
    pub repo_root: String,

    /// Pats to walk as "internal" source files
    pub root_paths: Vec<String>,

    /// packages we should consider as "entry" packages
    pub entry_packages: AHashSet<String>,

    /// Globs of individual files & directories to skip during the file walk.
    ///
    /// Some internal directories are always skipped.
    /// See [crate::walk::DEFAULT_SKIPPED_DIRS] for more details.
    pub skip: Vec<String>,
}

impl TryFrom<UnusedFinderJSONConfig> for UnusedFinderConfig {
    type Error = JsErr;
    fn try_from(value: UnusedFinderJSONConfig) -> std::result::Result<Self, Self::Error> {
        Ok(UnusedFinderConfig {
            // raw fields that are copied from the JSON config
            report_exported_symbols: value.report_exported_symbols,
            root_paths: value.root_paths,
            repo_root: value.repo_root,
            // other fields that are processed before use
            entry_packages: value.entry_packages.iter().cloned().collect(),
            skip: value.skip,
        })
    }
}
