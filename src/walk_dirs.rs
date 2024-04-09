use crate::fence::{parse_fence_file, Fence};
use crate::get_imports::get_imports_map_from_file;
use crate::path_utils::{get_slashed_path_buf, slashed_as_relative_path};
use jwalk::WalkDirGeneric;
use napi::bindgen_prelude::FromNapiValue;
use napi::bindgen_prelude::ToNapiValue;
use path_slash::PathExt;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
extern crate pathdiff;

fn should_retain_file(s: &str) -> bool {
    s == "fence.json"
        || s.ends_with(".ts")
        || s.ends_with(".tsx")
        || s.ends_with(".js")
        || s.ends_with(".jsx")
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct SourceFile {
    pub source_file_path: String,
    // ref to the strings of tags that apply to this file
    pub tags: HashSet<String>,
    pub imports: HashMap<String, Option<HashSet<String>>>,
}

#[derive(Eq, Debug, PartialEq)]
#[napi_derive::napi]
pub enum ExternalFences {
    Include = 0,
    Ignore = 1,
}

#[derive(Debug, Deserialize, PartialEq)]
pub enum WalkFileData {
    Fence(Fence),
    SourceFile(SourceFile),
    Nothing,
}

impl Default for WalkFileData {
    fn default() -> WalkFileData {
        WalkFileData::Nothing
    }
}

type TagList = HashSet<String>;

use lazy_static::lazy_static;
use std::env::current_dir;
lazy_static! {
    static ref WORKING_DIR_PATH: PathBuf = current_dir().unwrap();
}

pub fn discover_fences_and_files(
    start_path: &str,
    ignore_external_fences: ExternalFences,
    ignored_dirs: Vec<regex::Regex>,
) -> Vec<WalkFileData> {
    let walk_dir = WalkDirGeneric::<(TagList, WalkFileData)>::new(start_path).process_read_dir(
        move |read_dir_state, children| {
            children.iter_mut().for_each(|child| {
                if let Ok(dir_entry) = child {
                    if dir_entry.file_name() == "node_modules" || dir_entry.file_name() == "lib" {
                        dir_entry.read_children_path = None;
                    }
                }
            });
            // Custom filter -- retain only directories and fence.json files
            children.retain(|dir_entry_result| {
                match dir_entry_result {
                    Ok(dir_entry) => {
                        if let Some(slashed) = dir_entry.path().to_slash() {
                            return !(ignore_external_fences == ExternalFences::Ignore
                                && slashed.to_string().ends_with("/node_modules"))
                                && !ignored_dirs.iter().any(|d| d.is_match(&slashed));
                        }
                    }
                    Err(_) => {}
                }
                dir_entry_result
                    .as_ref()
                    .map(|dir_entry| match dir_entry.file_name.to_str() {
                        Some(file_name_str) => {
                            if dir_entry.file_type.is_dir() {
                                return true;
                            } else {
                                return should_retain_file(file_name_str);
                            }
                        }
                        None => return false,
                    })
                    .unwrap_or(false)
            });

            // Look for fence.json files and add their tags to the tag list for this walk
            for child_result in children.iter_mut() {
                match child_result {
                    Ok(dir_entry) => {
                        let f = dir_entry.file_name.to_str();
                        match f {
                            Some(file_name) => {
                                if file_name.ends_with("fence.json") {
                                    let _working_dir_path: &Path = &WORKING_DIR_PATH;
                                    let joined = &dir_entry.parent_path.join(file_name);
                                    let slashed = match get_slashed_path_buf(joined) {
                                        Ok(slashed) => slashed,
                                        Err(e) => {
                                            eprintln!("{}", e.to_string());
                                            continue;
                                        }
                                    };
                                    let fence_path = match slashed_as_relative_path(&slashed) {
                                        Ok(fence_path) => fence_path,
                                        Err(e) => {
                                            eprintln!("{}", e.to_string());
                                            continue;
                                        }
                                    };
                                    let fence_result =
                                        parse_fence_file(fence_path.as_relative_path());
                                    match fence_result {
                                        Ok(fence) => {
                                            // update fences
                                            let tags_clone = fence.fence.tags.clone();
                                            if tags_clone.is_some() {
                                                for tag in tags_clone.unwrap() {
                                                    read_dir_state.insert(tag);
                                                }
                                            }

                                            // fence.path_relative_to(&WORKING_DIR_PATH);
                                            // update client state from the walk
                                            dir_entry.client_state = WalkFileData::Fence(fence);
                                        }
                                        Err(error_message) => {
                                            println!("Error parsing fence!: {:}", error_message)
                                        }
                                    }
                                }
                            }
                            None => panic!("c_str was not a string?"),
                        }
                    }
                    // TODO maybe don't swallow errors here? not sure
                    // when this error even fires.
                    Err(_) => (),
                }
            }

            for child_result in children {
                match child_result {
                    Ok(dir_entry) => {
                        let f = dir_entry.file_name.to_str();
                        match f {
                            Some(file_name) => {
                                if file_name.ends_with(".ts")
                                    || file_name.ends_with(".tsx")
                                    || file_name.ends_with(".jsx")
                                    || file_name.ends_with(".js")
                                {
                                    let file_path = dir_entry.parent_path.join(file_name);
                                    let file_path = match get_slashed_path_buf(&file_path) {
                                        Ok(slashed) => slashed,
                                        Err(e) => {
                                            eprintln!("{}", e.to_string());
                                            continue;
                                        }
                                    };
                                    let _working_dir_path: &Path = &WORKING_DIR_PATH;

                                    let source_file_path = slashed_as_relative_path(&file_path);

                                    let source_file_path_str =
                                        source_file_path.unwrap().to_string();

                                    let imports = match get_imports_map_from_file(&file_path) {
                                        Ok(imps) => imps,
                                        Err(e) => {
                                            eprintln!("Error {}", e);
                                            continue;
                                        }
                                    };

                                    dir_entry.client_state = WalkFileData::SourceFile(SourceFile {
                                        source_file_path: source_file_path_str,
                                        imports,
                                        tags: HashSet::from_iter(
                                            read_dir_state.iter().map(|x| x.to_owned()),
                                        ),
                                    });
                                }
                            }
                            None => panic!("c_str was not a string?"),
                        }
                    }
                    // TODO maybe don't swallow errors here? not sure
                    // when this error even fires.
                    Err(_) => (),
                }
            }
        },
    );

    return walk_dir
        .into_iter()
        .filter(|e| e.is_ok())
        .map(|ok| ok.unwrap().client_state)
        .collect();
}

#[cfg(test)]
mod test {
    use crate::fence::{Fence, ParsedFence};
    use crate::walk_dirs::{discover_fences_and_files, SourceFile, WalkFileData};
    use std::collections::{HashMap, HashSet};
    use std::iter::{FromIterator, Iterator};

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

    macro_rules! set(
        { $($member:expr),+ } => {
            {
                HashSet::from_iter(vec!(
                    $(
                        String::from($member),
                    )+
                ))
            }
        };
    );

    #[test]
    fn test_simple_contains_root_fence() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected_root_fence = Fence {
            fence_path: "tests/walk_dir_simple/fence.json".to_owned(),
            fence: ParsedFence {
                tags: Some(vec![
                    "root-fence-tag-1".to_owned(),
                    "root-fence-tag-2".to_owned(),
                ]),
                exports: Option::None,
                dependencies: Option::None,
                imports: Option::None,
            },
        };

        assert!(
            discovered.iter().any(|x| match x {
                WalkFileData::Fence(y) => expected_root_fence == *y,
                _ => false,
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected_root_fence,
            discovered
        );
    }

    #[test]
    fn test_index_file() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "./tests/comments_panel_test",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected = "tests/comments_panel_test/packages/accelerator/accelerator-common/src/CommentsPanel/index.ts";
        assert!(
            discovered.iter().any(|file: &WalkFileData| {
                match file {
                    WalkFileData::SourceFile(source_file) => {
                        source_file.source_file_path == expected
                    }
                    _ => false,
                }
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected,
            discovered,
        );
    }

    #[test]
    fn test_simple_contains_subsubdir_fence() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected_subsubdir_fence = Fence {
            fence_path: "tests/walk_dir_simple/subdir/subsubdir/fence.json".to_owned(),
            fence: ParsedFence {
                tags: Some(vec!["subsubdir-fence-tag".to_owned()]),
                exports: Option::None,
                dependencies: Option::None,
                imports: Option::None,
            },
        };

        assert!(
            discovered.iter().any(|x| match x {
                WalkFileData::Fence(y) => expected_subsubdir_fence == *y,
                _ => false,
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected_subsubdir_fence,
            discovered
        );
    }

    #[test]
    fn test_simple_contains_root_file_imports() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected_root_ts_file = SourceFile {
            source_file_path: "tests/walk_dir_simple/rootFile.ts".to_owned(),
            tags: set!("root-fence-tag-1".to_owned(), "root-fence-tag-2".to_owned()),
            imports: map!(
              "root-ts-file-import-1" => Option::Some(set!("importFromRootFile"))
            ),
        };

        assert!(
            discovered.iter().any(|x| match x {
                WalkFileData::SourceFile(y) => expected_root_ts_file == *y,
                _ => false,
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected_root_ts_file,
            discovered
        );
    }

    #[test]
    fn test_simple_contains_sub_dir_file_imports() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected_subdir_ts_file = SourceFile {
            source_file_path: "tests/walk_dir_simple/subdir/subDirFile.ts".to_owned(),
            tags: set!("root-fence-tag-1".to_owned(), "root-fence-tag-2".to_owned()),
            imports: map!(
              "subdir-file-default-import" => Option::Some(set!("default")),
              "subdir-file-named-import" => Option::Some(set!("namedImport"))
            ),
        };

        assert!(
            discovered.iter().any(|x| match x {
                WalkFileData::SourceFile(y) => expected_subdir_ts_file == *y,
                _ => false,
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected_subdir_ts_file,
            discovered
        );
    }

    #[test]
    fn test_simple_contains_sub_sub_dir_file_imports() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            Vec::new(),
        );

        let expected_subdir_ts_file = SourceFile {
            source_file_path: "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts".to_owned(),
            tags: set!(
                "root-fence-tag-1".to_owned(),
                "root-fence-tag-2".to_owned(),
                "subsubdir-fence-tag".to_owned()
            ),
            imports: map!(
              "sub-sub-dir-file-abc-named-imports" => Option::Some(set!("a","b","c"))
            ),
        };

        assert!(
            discovered.iter().any(|x| match x {
                WalkFileData::SourceFile(y) => expected_subdir_ts_file == *y,
                _ => false,
            }),
            "expected discovered files to contain {:?}, but it did not. Actual: {:?}",
            expected_subdir_ts_file,
            discovered
        );
    }

    #[test]
    fn test_retrieve_nothing_on_ignored() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            vec![regex::Regex::new("tests/.**/subdir").unwrap()],
        );

        let expected_subdir_ts_file = SourceFile {
            source_file_path: "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts".to_owned(),
            tags: set!(
                "root-fence-tag-1".to_owned(),
                "root-fence-tag-2".to_owned(),
                "subsubdir-fence-tag".to_owned()
            ),
            imports: HashMap::new(),
        };

        assert!(
            !discovered.iter().any(|x| match x {
                WalkFileData::SourceFile(y) =>
                    *y.source_file_path
                        == "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts".to_owned(),
                _ => false,
            }),
            "Expected to have ignored {:?}, but it did not. Actual: {:?}",
            expected_subdir_ts_file,
            discovered
        );
    }

    #[test]
    fn test_simple_ignore_subsubdir_fence() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            vec![regex::Regex::new("tests/.**/subdir").unwrap()],
        );

        assert!(
            !discovered.iter().any(|x| match x {
                WalkFileData::Fence(y) =>
                    *y.fence_path == "tests/walk_dir_simple/subdir/subsubdir/fence.json".to_owned(),
                _ => false,
            }),
            "expected ignored files, got {:?}",
            discovered
        );
    }

    #[test]
    fn test_simple_ignore_subsu_fence() {
        let discovered: Vec<WalkFileData> = discover_fences_and_files(
            "tests/walk_dir_simple",
            crate::walk_dirs::ExternalFences::Ignore,
            vec![regex::Regex::new("^subdir").unwrap()],
        );

        let expected_subdir_fence = Fence {
            fence_path: "tests/walk_dir_simple/subdir/subsufence.json".to_owned(),
            fence: ParsedFence {
                tags: Some(vec!["subsubdir-fence-tag".to_owned()]),
                exports: Option::None,
                dependencies: Option::None,
                imports: Option::None,
            },
        };

        assert!(
            !discovered.iter().any(|x| match x {
                WalkFileData::Fence(y) => *y == expected_subdir_fence,
                _ => false,
            }),
            "expected find file, got {:?}",
            discovered
        );
    }
}
