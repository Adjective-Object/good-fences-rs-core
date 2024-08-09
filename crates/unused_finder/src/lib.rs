extern crate import_resolver;
extern crate serde_json;
extern crate tsconfig_paths;
#[macro_use]
extern crate anyhow;

mod api;
mod export_collector_tests;
pub mod graph;
pub mod import_export_info;
pub mod node_visitor;
pub mod unused_finder;
pub mod unused_finder_visitor_runner;
mod utils;
mod walked_file;

pub use api::{find_unused_items, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport};
