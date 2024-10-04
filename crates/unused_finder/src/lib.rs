#![feature(closure_lifetime_binder)]
#![feature(inherent_associated_types)]

extern crate import_resolver;
extern crate serde_json;
extern crate tsconfig_paths;

#[macro_use]
extern crate anyhow;

#[cfg(feature = "napi")]
#[macro_use]
extern crate napi_derive;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

#[cfg(test)]
#[macro_use]
extern crate test_tmpdir;

mod cfg;
mod core;
pub mod graph;
mod logger;
pub mod parse;
mod report;
pub mod unused_finder;
mod walk;
mod walked_file;

pub use core::{find_unused_items, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport};
pub use unused_finder::UnusedFinder;
