use crate::variables::VariableScope;
use oxc_span::Span;

use crate::raw_module_deps::RawModuleDeps;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    // The imports/exports extracted from the AST(s) of this segment
    pub module_deps: RawModuleDeps,
    // The name scope from traversing the segment
    pub variables: VariableScope,
    /// Disjoint ownership range: statement, split declarator, or opaque CJS body.
    /// Executable assembly uses `SegmentRecipe::source`, not this range alone.
    pub span: Span,
}
