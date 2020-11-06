extern crate find_ts_imports;
extern crate serde;
use crate::fence::{parse_fence_file, Fence};
use find_ts_imports::{parse_source_file_imports, SourceFileImportData};
use jwalk::WalkDirGeneric;
use serde::Deserialize;
use std::collections::HashSet;
use std::iter::FromIterator;

fn should_retain_file(s: &str) -> bool {
  s == "fence.json" || s.ends_with(".ts") || s.ends_with(".tsx")
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct SourceFile {
  pub source_file_path: String,
  // ref to the strings of tags that apply to this file
  pub tags: HashSet<String>,
  pub imports: SourceFileImportData,
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

pub fn discover_fences_and_files(start_path: &str) -> Vec<WalkFileData> {
  let walk_dir = WalkDirGeneric::<(TagList, WalkFileData)>::new(start_path).process_read_dir(
    |read_dir_state, children| {
      // Custom filter -- retain only directories and fence.json files
      children.retain(|dir_entry_result| {
        dir_entry_result
          .as_ref()
          .map(|dir_entry| {
            dir_entry.file_type.is_dir()
              || match dir_entry.file_name.to_str() {
                Some(file_name_str) => should_retain_file(file_name_str),
                None => false,
              }
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
                  let fence_result = parse_fence_file(&dir_entry.parent_path.join(file_name));
                  match fence_result {
                    Ok(fence) => {
                      // update fences
                      let tags_clone = fence.fence.tags.clone();
                      if tags_clone.is_some() {
                        for tag in tags_clone.unwrap() {
                          read_dir_state.insert(tag);
                        }
                      }

                      // update client state from the walk
                      dir_entry.client_state = WalkFileData::Fence(fence);
                    }
                    Err(error_message) => println!("Error parsing fence!: {:}", error_message),
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
                if file_name.ends_with(".ts") || file_name.ends_with(".tsx") {
                  let file_path = dir_entry.parent_path.join(file_name);
                  dir_entry.client_state = WalkFileData::SourceFile(SourceFile {
                    source_file_path: file_path.to_str().unwrap().to_owned(),
                    imports: parse_source_file_imports(&file_path),
                    tags: HashSet::from_iter(read_dir_state.iter().map(|x| x.to_owned())),
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
  use find_ts_imports::SourceFileImportData;
  use std::collections::HashSet;
  use std::env::current_dir;
  use std::iter::{FromIterator, Iterator};
  use std::path::PathBuf;

  fn force_to_abs_path_str(p: &str) -> String {
    let x = PathBuf::from(p);
    return x.canonicalize().unwrap().to_str().unwrap().to_owned();
  }

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
    let mut test_path_buf = current_dir().unwrap();
    test_path_buf.push("walk_dir_tests");
    test_path_buf.push("simple");
    let current_dir_str = current_dir().unwrap();

    let discovered: Vec<WalkFileData> = discover_fences_and_files(test_path_buf.to_str().unwrap());

    let expected_root_fence = Fence {
      fence_path: force_to_abs_path_str("walk_dir_tests/simple/fence.json"),
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
  fn test_simple_contains_subsubdir_fence() {
    let mut test_path_buf = current_dir().unwrap();
    test_path_buf.push("walk_dir_tests");
    test_path_buf.push("simple");
    let current_dir_str = current_dir().unwrap();

    let discovered: Vec<WalkFileData> = discover_fences_and_files(test_path_buf.to_str().unwrap());

    let expected_subsubdir_fence = Fence {
      fence_path: force_to_abs_path_str("walk_dir_tests/simple/subdir/subsubdir/fence.json"),
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
    let mut test_path_buf = current_dir().unwrap();
    test_path_buf.push("walk_dir_tests");
    test_path_buf.push("simple");
    let current_dir_str = current_dir().unwrap();

    let discovered: Vec<WalkFileData> = discover_fences_and_files(test_path_buf.to_str().unwrap());

    let expected_root_ts_file = SourceFile {
      source_file_path: force_to_abs_path_str("walk_dir_tests/simple/rootFile.ts"),
      tags: set!("root-fence-tag-1".to_owned(), "root-fence-tag-2".to_owned()),
      imports: SourceFileImportData {
        imports: map!(
          "root-ts-file-import-1" => Option::Some(set!("importFromRootFile"))
        ),
      },
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
    let mut test_path_buf = current_dir().unwrap();
    test_path_buf.push("walk_dir_tests");
    test_path_buf.push("simple");
    let current_dir_str = current_dir().unwrap();

    let discovered: Vec<WalkFileData> = discover_fences_and_files(test_path_buf.to_str().unwrap());

    let expected_subdir_ts_file = SourceFile {
      source_file_path: force_to_abs_path_str("walk_dir_tests/simple/subdir/subDirFile.ts"),
      tags: set!("root-fence-tag-1".to_owned(), "root-fence-tag-2".to_owned()),
      imports: SourceFileImportData {
        imports: map!(
          "subdir-file-default-import" => Option::Some(set!("default")),
          "subdir-file-named-import" => Option::Some(set!("namedImport"))
        ),
      },
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
    let mut test_path_buf = current_dir().unwrap();
    test_path_buf.push("walk_dir_tests");
    test_path_buf.push("simple");
    let current_dir_str = current_dir().unwrap();

    let discovered: Vec<WalkFileData> = discover_fences_and_files(test_path_buf.to_str().unwrap());

    let expected_subdir_ts_file = SourceFile {
      source_file_path: force_to_abs_path_str(
        "walk_dir_tests/simple/subdir/subsubdir/subSubDirFile.ts",
      ),
      tags: set!(
        "root-fence-tag-1".to_owned(),
        "root-fence-tag-2".to_owned(),
        "subsubdir-fence-tag".to_owned()
      ),
      imports: SourceFileImportData {
        imports: map!(
          "sub-sub-dir-file-abc-named-imports" => Option::Some(set!("a","b","c"))
        ),
      },
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
}
