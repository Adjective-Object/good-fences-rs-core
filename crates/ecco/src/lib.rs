//! Owned effect declarations, semantic JavaScript matching, and parallel propagation.
#![doc = include_str!("../README.md")]
mod javascript;
mod language;
mod parser;
mod solver;

pub use javascript::{
    analyze_source, extract_from_ast, DefinitionMatch, SourceAnalysis, SourceRoot,
};
pub use language::*;
pub use parser::parse_declarations;
pub use solver::{CallResult, Context, Effect, Evaluation, ValueId};

#[cfg(test)]
mod tests;
