//! Owned effect declarations, semantic JavaScript matching, and parallel propagation.
#![doc = include_str!("../README.md")]
mod analysis;
mod environment;
mod javascript;
mod language;
mod parser;
mod solver;

pub use analysis::{
    Analysis, AnalysisResult, ExternalReference, FileAnalysis, ParsedFile, ParsedRoot,
    ParsedSymbol, Resolution, Resolver, RootId, Symbol, SymbolId,
};
pub use environment::{declaration_matches, DeclarationConfig};
pub use javascript::{
    analyze_source, extract_from_ast, parse_from_ast, parse_from_ast_with_segments, parse_source,
    DefinitionMatch, SourceAnalysis, SourceRoot,
};
pub use language::*;
pub use parser::parse_declarations;
pub use solver::{CallResult, Context, Effect, Evaluation, OwnedFunction, ValueId};

#[cfg(test)]
mod tests;
