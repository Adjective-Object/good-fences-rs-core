extern crate import_resolver;
extern crate tsconfig_paths;
#[macro_use]
extern crate anyhow;

mod export_collector_tests;
pub mod graph;
pub mod node_visitor;
pub mod unused_finder;
pub mod unused_finder_visitor_runner;
pub mod import_export_info;
mod utils;
mod walked_file;
mod api;

pub use api::{
    find_unused_items,
    FindUnusedItemsConfig,
    ExportedItemReport,
    UnusedFinderReport,
};
