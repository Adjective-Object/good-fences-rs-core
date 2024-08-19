extern crate import_resolver;
extern crate serde_json;
extern crate tsconfig_paths;

#[macro_use]
extern crate anyhow;

#[cfg(feature = "napi")]
#[macro_use]
extern crate napi_derive;

#[cfg_attr(test, macro_use)]
extern crate pretty_assertions;

mod core;
mod export_collector_tests;
mod fixer;
pub mod graph;
pub mod import_export_info;
pub mod node_visitor;
pub mod unused_finder;
pub mod unused_finder_visitor_runner;
mod utils;
mod walked_file;

pub use core::{find_unused_items, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport};
pub use unused_finder::UnusedFinder;
