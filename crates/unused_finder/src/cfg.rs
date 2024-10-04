use ahashmap::AHashSet;
use anyhow::{Context, Result};
use js_err::JsErr;
use serde::Deserialize;
use std::{str::FromStr, sync::Arc};

/// A JSON serializable proxy for the UnusedItemsConfig struct
///
/// This struct is used to serialize the UnusedItemsConfig struct to JSON
/// with serde, or to recieve the config to JS via napi.
#[cfg_attr(feature = "napi", napi(object))]
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedItemsJSONConfig {
    // Trace exported symbols that are not imported anywhere in the project
    #[serde(default)]
    pub report_exported_items: bool,
    // Root paths to walk as source files
    #[serde(alias = "pathsToRead")]
    pub root_paths: Vec<String>,
    // Path to the root directory of the repository.
    pub repo_root: String,
    // Files under matching dirs won't be scanned.
    pub skipped_dirs: Vec<String>,
    // List of regex. Named items in the form of `export { foo }` and similar (excluding `default`) matching a regex in this list will not be recorded as imported/exported items.
    // e.g. skipped_items = [".*Props$"] and a file contains a `export type FooProps = ...` statement, FooProps will not be recorded as an exported item.
    // e.g. skipped_items = [".*Props$"] and a file contains a `import { BarProps } from 'bar';` statement, BarProps will not be recorded as an imported item.
    pub skipped_items: Vec<String>,
    pub entry_packages: Vec<String>,
}

/// Configuration for the unused items finder
#[derive(Debug, Default, Clone)]
pub struct UnusedItemsConfig {
    // If true, the finder should report exported symbols that are not used anywhere in the project
    pub report_exported_items: bool,

    // Path to the root directory of the repository
    pub repo_root: String,

    // Pats to walk as "internal" source files
    pub root_paths: Vec<String>,

    // Path to the root directory of the repository.
    pub entry_packages: AHashSet<String>,
    pub skipped_items: Arc<Vec<regex::Regex>>,
    pub skipped_dirs: Arc<Vec<glob::Pattern>>,
}

impl TryFrom<UnusedItemsJSONConfig> for UnusedItemsConfig {
    type Error = JsErr;
    fn try_from(value: UnusedItemsJSONConfig) -> std::result::Result<Self, Self::Error> {
        let skipped_items = value
            .skipped_items
            .iter()
            .map(|s| regex::Regex::from_str(s.as_str()))
            .collect::<Result<Vec<regex::Regex>, _>>()
            .context("while parsing skipped_items as regexp")
            .map_err(JsErr::invalid_arg)?;

        let skipped_dirs = value
            .skipped_dirs
            .iter()
            .map(|s| glob::Pattern::new(s))
            .collect::<Result<Vec<glob::Pattern>, _>>()
            .context("while parsing skipped_dirs as glob patterns")
            .map_err(JsErr::invalid_arg)?;

        Ok(UnusedItemsConfig {
            // raw fields that are copied from the JSON config
            report_exported_items: value.report_exported_items,
            root_paths: value.root_paths,
            repo_root: value.repo_root,
            // other fields that are processed before use
            entry_packages: value.entry_packages.iter().cloned().collect(),
            skipped_items: Arc::new(skipped_items),
            skipped_dirs: Arc::new(skipped_dirs),
        })
    }
}

// Looks in cwd for a file called `.unusedignore`
// allowed items can be:
// - specific file paths like `shared/internal/owa-react-hooks/src/useWhyDidYouUpdate.ts`
// - glob patterns (similar to a `.gitignore` file) `shared/internal/owa-datetime-formatters/**`
pub fn read_allow_list() -> Result<Vec<glob::Pattern>> {
    return match std::fs::read_to_string(".unusedignore") {
        Ok(list) => list
            .split("\n")
            .enumerate()
            .map(|(idx, line)| {
                glob::Pattern::new(line)
                    .map_err(|e| anyhow!("line {}: failed to parse pattern: {}", idx, e))
            })
            .collect::<Result<Vec<glob::Pattern>, anyhow::Error>>(),
        Err(e) => Err(anyhow!("failed to read .unusedignore file: {}", e)),
    };
}
