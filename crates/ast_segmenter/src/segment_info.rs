use ast_name_tracker::VariableScope;
use swc_common::Span;

use crate::raw_module_deps::RawModuleDeps;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSegment {
    // The imports/exports extracted from the AST(s) of this segment
    pub module_deps: RawModuleDeps,
    // The name scope from traversing the segment
    pub variables: VariableScope,
    /// The source span of the top-level ModuleItem this segment was created from.
    pub span: Span,
}
