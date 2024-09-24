use crate::fence::{parse_fence_file, Fence};
use crate::get_imports::get_imports_map_from_file;
use anyhow::{anyhow, Error, Result};
use jwalk::WalkDirGeneric;
use path_slash::PathExt;
use path_utils::as_relative_slash_path;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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

#[derive(Debug, Deserialize, PartialEq, Default)]
pub enum WalkFileData {
    Fence(Fence),
    SourceFile(SourceFile),
    #[default]
    Nothing,
}

type TagList = HashSet<String>;

use lazy_static::lazy_static;
use std::env::current_dir;
lazy_static! {
    static ref WORKING_DIR_PATH: PathBuf = current_dir().unwrap();
}

fn discover_js_ts_src(file_path: &PathBuf, tags: HashSet<String>) -> Result<WalkFileData, Error> {
    let relative_file_path = as_relative_slash_path(file_path)?;
    let imports = get_imports_map_from_file(&relative_file_path)
        .map_err(|e| anyhow!("Error getting imports from file {:?}: {}", file_path, e))?;

    Ok(WalkFileData::SourceFile(SourceFile {
        source_file_path: relative_file_path.into_string(),
        imports,
        tags,
    }))
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
                if let Ok(dir_entry) = dir_entry_result {
                    if let Some(slashed) = dir_entry.path().to_slash() {
                        return (ignore_external_fences != ExternalFences::Ignore
                            || !slashed.to_string().ends_with("/node_modules"))
                            && !ignored_dirs.iter().any(|d| d.is_match(&slashed));
                    }
                }
                dir_entry_result
                    .as_ref()
                    .map(|dir_entry| match dir_entry.file_name.to_str() {
                        Some(file_name_str) => {
                            if dir_entry.file_type.is_dir() {
                                true
                            } else {
                                should_retain_file(file_name_str)
                            }
                        }
                        None => false,
                    })
                    .unwrap_or(false)
            });

            // Look for fence.json files and add their tags to the tag list for this walk
            //
            // We do this in a separate iteration from the one below, because we need to ensure tags
            // are applied to the walk's taglist before we start processing the files.
            for child_result in children.iter_mut() {
                match child_result {
                    Ok(dir_entry) => {
                        let f = dir_entry.file_name.to_str();
                        match f {
                            Some(file_name) => {
                                if file_name.ends_with("fence.json") {
                                    let fence_path = &dir_entry.parent_path.join(file_name);
                                    let parsed_fence: Result<Fence, _> =
                                        as_relative_slash_path(fence_path)
                                            .and_then(parse_fence_file);
                                    let fence = match parsed_fence {
                                        Ok(fence) => fence,
                                        Err(e) => {
                                            eprintln!("{}", e);
                                            continue;
                                        }
                                    };
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
                            }
                            None => panic!(
                                "c_str was not a string?: {}",
                                dir_entry.file_name.to_string_lossy()
                            ),
                        }
                    }
                    Err(e) => {
                        eprintln!("Unknown Walk Error {}", e);
                        continue;
                    }
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
                                    let file_path: PathBuf = dir_entry.parent_path.join(file_name);
                                    dir_entry.client_state = match discover_js_ts_src(
                                        &file_path,
                                        read_dir_state.clone(),
                                    ) {
                                        Ok(sf) => sf,
                                        Err(e) => {
                                            eprintln!("Error {}", e);
                                            continue;
                                        }
                                    };
                                }
                            }
                            None => panic!(
                                "c_str was not a string?: {}",
                                dir_entry.file_name.to_string_lossy()
                            ),
                        }
                    }
                    // TODO maybe don't swallow errors here? not sure
                    // when this error even fires.
                    Err(e) => {
                        eprintln!("Unknown Walk Error {}", e);
                        continue;
                    }
                }
            }
        },
    );

    walk_dir
        .into_iter()
        .filter(|e| e.is_ok())
        .map(|ok| ok.unwrap().client_state)
        .collect()
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
                    y.source_file_path == "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts",
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
                    y.fence_path == "tests/walk_dir_simple/subdir/subsubdir/fence.json",
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
