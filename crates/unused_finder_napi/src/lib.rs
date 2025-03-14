use std::collections::HashMap;

use js_err_napi::ToNapi;
use napi::{JsObject, Result};

use logger_console::ConsoleLogger;
use napi_derive::napi;

/// A JSON serializable proxy for the UnusedFinderConfig struct
///
/// This struct is used to deserialize the UnusedFinderConfig struct
/// from a config file to with serde / over the debug bridge for napi
#[derive(Debug, Default, Clone)]
#[napi(object)]
pub struct UnusedFinderJSONConfig {
    /// Path to the root directory of the repository.
    pub repo_root: String,
    /// Root paths to walk as source files
    ///
    /// These can be either absolute paths, or paths relative to the repo root
    pub root_paths: Vec<String>,
    /// A List of globs.
    /// Matching files and directories won't be scanned during the file walk
    ///
    /// Matches are made against the names of the individual directories,
    /// NOT the full directory paths
    pub skip: Option<Vec<String>>,
    /// If true, individual exported symbols are also tracked
    pub report_exported_symbols: Option<bool>,
    pub allow_unused_types: Option<bool>,
    /// List of packages that should be considered "entry" packages
    /// All transitive imports from the exposed exports of these packages
    /// will be considered used
    ///
    /// Items are parsed in one of three ways:
    /// 1. If the item starts with "./", it is treated as a path glob, and evaluated against the paths of package folders, relative to the repo root.
    /// 2. If the item contains any of "~)('!*", it is treated as a name-glob, and evaluated as a glob against the names of packages.
    /// 3. Otherwise, the item is treated as the name of an individual package, and matched literally.
    pub entry_packages: Vec<String>,
    /// List of glob patterns to mark as "tests".
    /// These files will be marked as used, and all of their transitive
    /// dependencies will also be marked as used
    ///
    /// glob patterns are matched against the relative file path from the
    /// root of the repository
    pub test_files: Option<Vec<String>>,
}

impl From<UnusedFinderJSONConfig> for unused_finder::UnusedFinderJSONConfig {
    fn from(val: UnusedFinderJSONConfig) -> Self {
        unused_finder::UnusedFinderJSONConfig {
            repo_root: val.repo_root,
            root_paths: val.root_paths,
            skip: val.skip.unwrap_or_default(),
            report_exported_symbols: val.report_exported_symbols.unwrap_or_default(),
            entry_packages: val.entry_packages,
            allow_unused_types: val.allow_unused_types.unwrap_or_default(),
            test_files: val.test_files.unwrap_or_default(),
        }
    }
}

#[derive(Debug, PartialEq, Ord, PartialOrd, Eq)]
#[napi(string_enum)]
pub enum UsedTagEnum {
    Entry,
    Ignored,
    TypeOnly,
    Test,
}

impl From<unused_finder::UsedTagEnum> for UsedTagEnum {
    fn from(val: unused_finder::UsedTagEnum) -> Self {
        match val {
            unused_finder::UsedTagEnum::Entry => UsedTagEnum::Entry,
            unused_finder::UsedTagEnum::Ignored => UsedTagEnum::Ignored,
            unused_finder::UsedTagEnum::TypeOnly => UsedTagEnum::TypeOnly,
            unused_finder::UsedTagEnum::Test => UsedTagEnum::Test,
        }
    }
}

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq)]
#[napi(object)]
pub struct SymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

impl From<unused_finder::SymbolReport> for SymbolReport {
    fn from(val: unused_finder::SymbolReport) -> Self {
        SymbolReport {
            id: val.id,
            start: val.start,
            end: val.end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq)]
#[napi(object)]
pub struct SymbolReportWithTags {
    pub symbol: SymbolReport,
    pub tags: Vec<UsedTagEnum>,
}

impl From<unused_finder::SymbolReportWithTags> for SymbolReportWithTags {
    fn from(val: unused_finder::SymbolReportWithTags) -> Self {
        SymbolReportWithTags {
            symbol: val.symbol.into(),
            tags: val.tags.into_iter().map(Into::into).collect(),
        }
    }
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default, PartialEq)]
#[napi]
pub struct UnusedFinderReport {
    // files that are completely unused
    pub unused_files: Vec<String>,
    // items that are unused within files
    // note that this intentionally uses a std HashMap type to guarantee napi
    // compatibility
    pub unused_symbols: HashMap<String, Vec<SymbolReport>>,
    pub extra_file_tags: HashMap<String, Vec<UsedTagEnum>>,
    pub extra_symbol_tags: HashMap<String, Vec<SymbolReportWithTags>>,
}

impl From<unused_finder::UnusedFinderReport> for UnusedFinderReport {
    fn from(val: unused_finder::UnusedFinderReport) -> Self {
        UnusedFinderReport {
            unused_files: val.unused_files,
            unused_symbols: val
                .unused_symbols
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(Into::into).collect()))
                .collect(),
            extra_file_tags: val
                .extra_file_tags
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(Into::into).collect()))
                .collect(),
            extra_symbol_tags: val
                .extra_symbol_tags
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(Into::into).collect()))
                .collect(),
        }
    }
}

// Holds an in-memory representation of the file tree.
// That representation can be used used to find unused files and exports
// within a project
//
// To use, create a new UnusedFinder, then call `find_unused` to get the accounting
// of unused files and exports.
#[napi]
pub struct UnusedFinder {
    inner: napi::Result<(ConsoleLogger, unused_finder::UnusedFinder)>,
}

#[napi]
impl UnusedFinder {
    #[napi(constructor)]
    pub fn new(console: JsObject, config: UnusedFinderJSONConfig) -> Self {
        Self {
            inner: Self::new_inner(console, config),
        }
    }

    fn new_inner(
        console: JsObject,
        config: UnusedFinderJSONConfig,
    ) -> napi::Result<(ConsoleLogger, unused_finder::UnusedFinder)> {
        // unpack the console logger
        let logger = ConsoleLogger::new(console)?;

        let inner =
            unused_finder::UnusedFinder::new_from_json_config(&logger, config.into()).into_napi();
        inner.map(|other_inner| (logger, other_inner))
    }

    pub fn mark_dirty(&mut self, file_paths: Vec<String>) -> napi::Result<()> {
        match &mut self.inner {
            Ok(ref mut inner) => {
                inner.1.mark_dirty(file_paths);
                Ok(())
            }
            Err(e) => Err(e.clone()),
        }
    }

    pub fn mark_all_dirty(&mut self) -> napi::Result<()> {
        match &mut self.inner {
            Ok(ref mut inner) => {
                inner.1.mark_all_dirty();
                Ok(())
            }
            Err(e) => Err(e.clone()),
        }
    }

    pub fn find_unused(&mut self) -> Result<UnusedFinderReport> {
        match &mut self.inner {
            Ok(ref mut inner) => {
                let result = inner.1.find_unused(&inner.0);
                result.into_napi().map(|result| result.get_report().into())
            }
            Err(e) => Err(e.clone()),
        }
    }
}

#[napi]
pub fn find_unused_items(
    console: JsObject,
    config: UnusedFinderJSONConfig,
) -> napi::Result<UnusedFinderReport> {
    let console_logger = ConsoleLogger::new(console)?;
    let result = unused_finder::find_unused_items(console_logger, config.into());
    match result {
        Ok(report) => Ok(report.into()),
        Err(err) => Err(err.into_napi()),
    }
}
