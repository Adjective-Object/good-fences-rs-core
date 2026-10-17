use crate::variables::VariableScope;
use oxc_span::Span;

use crate::raw_module_deps::RawModuleDeps;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    // The imports/exports extracted from the AST(s) of this segment
    pub module_deps: RawModuleDeps,
    // The name scope from traversing the segment
    pub variables: VariableScope,
    /// Zero-based, end-exclusive UTF-8 byte range of the top-level statement.
    pub span: Span,
}
