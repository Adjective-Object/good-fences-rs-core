// For paths processing in tsconfig
#![feature(iterator_try_collect)]



extern crate anyhow;
extern crate copy_from_str;
extern crate dashmap;
extern crate parking_lot;
extern crate path_clean;
extern crate pathdiff;
#[cfg(test)]
extern crate pretty_assertions;
extern crate relative_path;
extern crate serde;
extern crate serde_json;
extern crate swc_common;
extern crate swc_core;
extern crate swc_ecma_loader;
#[cfg_attr(test, macro_use)]
extern crate test_tmpdir;
extern crate tracing;
#[cfg(test)]
extern crate tracing_test;

pub mod manual_resolver;
pub mod swc_resolver;
