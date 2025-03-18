use ast_name_tracker::VariableScope;

use crate::raw_module_deps::RawModuleDeps;

pub struct RawSegment {
    // The imports/exports extracted from the AST(s) of this segment
    pub module_deps: RawModuleDeps,
    // The name scope from traversing the segment
    pub variables: VariableScope,
}
