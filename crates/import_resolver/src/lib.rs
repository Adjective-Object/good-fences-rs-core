// For paths processing in tsconfig
#![feature(iterator_try_collect)]

#[cfg(test)]
extern crate pretty_assertions;

pub mod manual_resolver;
pub mod resolve;
pub mod swc_resolver;
