#[macro_use]
extern crate serde;
extern crate serde_json;
extern crate thiserror;

mod error;
mod tsconfig_paths_json;

pub use tsconfig_paths_json::{TsconfigPathsJson, TsconfigPathsCompilerOptions};