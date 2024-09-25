use crate::fence::Fence;
use lazy_static::__Deref;
use path_slash::PathBufExt;
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

            if let Some(key_str) = key.to_slash() {
                let fence_option = self.fences_map.get(key_str.deref());

                if let Some(fence) = fence_option {
                    fences.push(fence)
                }
            }
        }
        fences
    }
}

#[cfg(test)]
mod test {
    use relative_path::RelativePathBuf;
    use std::path::Path;

    use super::FenceCollection;
    use crate::fence::parse_fence_str;

    macro_rules! map(
        { $($key:expr => $value:expr),+ } => {
            {
                let mut m = ::std::collections::HashMap::new();
                $(
                    m.insert(String::from($key), $value);
                )+
                m
            }
        };
    );

    #[test]
    fn test_get_fences_for_path() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "some/a/b/fence.json" => parse_fence_str(
                    r#"{"tags": ["b"]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap(),
                "some/fence.json" =>  parse_fence_str(r#"{"tags": ["root"]}"#, &RelativePathBuf::from("path/to/protected/fence.json")).unwrap(),
                "some/other/fence.json" =>  parse_fence_str(r#"{"tags": ["other"]}"#, &RelativePathBuf::from("path/to/protected/fence.json")).unwrap()
            ),
        };

        assert_eq!(
            fence_collection.get_fences_for_path(Path::new("some/file.ts")),
            vec![fence_collection.fences_map.get("some/fence.json").unwrap(),],
        );

        assert_eq!(
            fence_collection.get_fences_for_path(Path::new("some/other/file.ts")),
            vec![
                fence_collection
                    .fences_map
                    .get("some/other/fence.json")
                    .unwrap(),
                fence_collection.fences_map.get("some/fence.json").unwrap(),
            ],
            "should return multiple fences for file with multiple fences",
        );
    }
}
