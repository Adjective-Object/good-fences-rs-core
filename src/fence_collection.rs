use crate::fence::Fence;
use crate::path_utils::as_slashed_pathbuf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use lazy_static::__Deref;
use path_slash::{PathBufExt};

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
            
            match key.to_slash() {
                Some(key_str) => {
                    let fence_option = self.fences_map.get(key_str.deref());

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


#[cfg(test)]
mod test {
    use std::path::Path;
    use relative_path::RelativePathBuf;

    use crate::fence::parse_fence_str;
    use super::FenceCollection;


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
                "some/fucking/fence.json" =>  parse_fence_str(r#"{"tags": ["fucking"]}"#, &RelativePathBuf::from("path/to/protected/fence.json")).unwrap()
            )
        };

        assert_eq!(
            fence_collection.get_fences_for_path(&Path::new("some/file.ts")),
            vec![
                fence_collection.fences_map.get("some/fence.json").unwrap(),
            ],
        );

        assert_eq!(
            fence_collection.get_fences_for_path(&Path::new("some/fucking/file.ts")),
            vec![
                fence_collection.fences_map.get("some/fucking/fence.json").unwrap(),
                fence_collection.fences_map.get("some/fence.json").unwrap(),
            ],
            "should return multiple fences for file with multiple fences",
        );
    }
}