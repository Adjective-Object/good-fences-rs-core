use crate::fence::Fence;
use crate::walk_dirs::SourceFile;
use std::collections::HashMap;

struct FenceCollection {
    fences: HashMap<String, Fence>,
}
