use crate::fence::Fence;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub struct FenceCollection {
    pub fences_map: HashMap<String, Fence>,
}

impl FenceCollection {
    // TODO rewrite this as a generator?
    pub fn get_fences_for_path<'b>(&'b self, path: &Path) -> Vec<&'b Fence> {
        let mut fences: Vec<&'b Fence> = Vec::with_capacity(5);
        for stub in path.ancestors() {
            let mut key = PathBuf::from(stub);
            key.push("fence.json");

            match key.to_str() {
                Some(key_str) => {
                    let fence_option = self.fences_map.get(key_str);

                    match fence_option {
                        Some(fence) => fences.push(fence),
                        None => {}
                    }
                }
                None => {}
            }
        }
        fences
    }
}
