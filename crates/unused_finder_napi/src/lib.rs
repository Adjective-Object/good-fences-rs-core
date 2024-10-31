use std::{collections::HashMap, sync::Arc};

use js_err_napi::ToNapi;
use napi::{
    threadsafe_function::{
        ErrorStrategy::{self},
        ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
    },
    JsFunction, JsObject, Result, Status,
};

use napi_derive::napi;
use serde::{Deserialize, Serialize};
use unused_finder::logger::Logger;

/// A JSON serializable proxy for the UnusedFinderConfig struct
///
/// This struct is used to deserialize the UnusedFinderConfig struct
/// from a config file to with serde / over the debug bridge for napi
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[napi(object)]
pub struct UnusedFinderJSONConfig {
    /// Path to the root directory of the repository.
    #[serde(default)]
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
    #[serde(default)]
    pub skip: Vec<String>,
    /// If true, individual exported symbols are also tracked
    #[serde(default)]
    pub report_exported_symbols: bool,
    /// List of packages that should be considered "entry" packages
    /// All transitive imports from the exposed exports of these packages
    /// will be considered used
    ///
    /// Items are parsed in one of three ways:
    /// 1. If the item starts with "./", it is treated as a path glob, and evaluated against the paths of package folders, relative to the repo root.
    /// 2. If the item contains any of "~)('!*", it is treated as a name-glob, and evaluated as a glob against the names of packages.
    /// 3. Otherwise, the item is treated as the name of an individual package, and matched literally.
    pub entry_packages: Vec<String>,
}

impl From<UnusedFinderJSONConfig> for unused_finder::UnusedFinderJSONConfig {
    fn from(val: UnusedFinderJSONConfig) -> Self {
        unused_finder::UnusedFinderJSONConfig {
            repo_root: val.repo_root,
            root_paths: val.root_paths,
            skip: val.skip,
            report_exported_symbols: val.report_exported_symbols,
            entry_packages: val.entry_packages,
        }
    }
}

#[derive(Debug, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
#[napi(string_enum)]
pub enum UsedTagEnum {
    #[serde(rename = "entry")]
    Entry,
    #[serde(rename = "ignored")]
    Ignored,
}

impl From<unused_finder::UsedTagEnum> for UsedTagEnum {
    fn from(val: unused_finder::UsedTagEnum) -> Self {
        match val {
            unused_finder::UsedTagEnum::Entry => UsedTagEnum::Entry,
            unused_finder::UsedTagEnum::Ignored => UsedTagEnum::Ignored,
        }
    }
}

// Report of a single exported item in a file
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Serialize, Deserialize)]
#[napi(object)]
pub struct SymbolReport {
    pub id: String,
    pub start: u32,
    pub end: u32,
    pub tags: Option<Vec<UsedTagEnum>>,
}

impl From<unused_finder::SymbolReport> for SymbolReport {
    fn from(val: unused_finder::SymbolReport) -> Self {
        SymbolReport {
            id: val.id,
            start: val.start,
            end: val.end,
            tags: val
                .tags
                .map(|tags| tags.into_iter().map(Into::into).collect()),
        }
    }
}

// Report of unused symbols within a project
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[napi]
pub struct UnusedFinderReport {
    // files that are completely unused
    pub unused_files: Vec<String>,
    // items that are unused within files
    // note that this intentionally uses a std HashMap type to guarantee napi
    // compatibility
    pub unused_symbols: HashMap<String, Vec<SymbolReport>>,
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
        }
    }
}

#[derive(Clone)]
struct ConsoleLogger {
    logfn: Arc<ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>>,
}

impl ConsoleLogger {
    fn new(console: JsObject) -> Result<Self> {
        let logfn = console.get_named_property::<JsFunction>("log")?;
        Ok(Self {
            logfn: Arc::new(logfn.create_threadsafe_function(
                // allow queueing console responses?
                100,
                |ctx: ThreadSafeCallContext<String>| {
                    let js_str = ctx.env.create_string(&ctx.value)?;
                    // return as an argv array
                    Ok(vec![js_str])
                },
            )?),
        })
    }
}

impl Logger for ConsoleLogger {
    fn log(&self, message: impl Into<String>) {
        let message_string: String = message.into();
        let status = self
            .logfn
            .call(Ok(message_string), ThreadsafeFunctionCallMode::Blocking);
        match status {
            Status::Ok => {}
            _ => {
                eprintln!();
                panic!("Error calling console.log from Rust. Unexpected threadsafe function call mode {}", status);
            }
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
