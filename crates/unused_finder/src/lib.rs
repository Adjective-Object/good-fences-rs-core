#![feature(closure_lifetime_binder)]

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
pub mod graph;
pub mod parse;
pub mod unused_finder;
mod utils;
mod walked_file;

pub use core::{find_unused_items, ExportedItemReport, FindUnusedItemsConfig, UnusedFinderReport};
pub use unused_finder::UnusedFinder;
