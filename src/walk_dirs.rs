extern crate find_ts_imports;
extern crate serde;
extern crate swc_common;
extern crate swc_ecma_parser;
use crate::fence::{parse_fence_file, Fence};
use find_ts_imports::{parse_source_file_imports, SourceFileImportData};
use jwalk::{WalkDirGeneric, Error};
use relative_path::RelativePath;
use serde::Deserialize;
use swc_common::sync::Lrc;
use swc_common::{
    errors::{ColorConfig, Handler},
    FileName, FilePathMapping, SourceMap,
};
use swc_ecma_parser::Capturing;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
use std::collections::HashSet;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

use lazy_static::lazy_static;
use std::env::current_dir;
lazy_static! {
  static ref WORKING_DIR_PATH: PathBuf = current_dir().unwrap();
}

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
                  let _working_dir_path: &Path = &WORKING_DIR_PATH;
                  let fence_result = parse_fence_file(
                    RelativePath::from_path(&dir_entry.parent_path.join(file_name)).unwrap()
                  );
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
                if file_name.ends_with(".ts")
                  || file_name.ends_with(".tsx")
                  || file_name.ends_with(".jsx")
                  || file_name.ends_with(".js")
                {
                  let file_path = dir_entry.parent_path.join(file_name);
                  let _working_dir_path: &Path = &WORKING_DIR_PATH;
                  let source_file_path = RelativePath::from_path(&file_path);

                 
                  dir_entry.client_state = WalkFileData::SourceFile(SourceFile {
                    source_file_path: source_file_path.unwrap().to_string(),
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

pub fn create_lexer<'a>(fm: &'a swc_common::SourceFile) -> Lexer<'a, StringInput<'a>> {
    let lexer = Lexer::new(
      Syntax::Typescript(Default::default()),
      Default::default(),
      StringInput::from(fm),
      None
    );
    lexer
}

pub fn print_imports_from_swc<'a>(file_path: &'a PathBuf) {
  let cm = Arc::<SourceMap>::default();
  let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
  let fm = cm.load_file(Path::new(file_path.to_str().unwrap())).expect("Could not load file");
  let lexer = create_lexer(&fm);

  let capturing = Capturing::new(lexer);

  let mut parser = Parser::new_from(capturing);

  for e in parser.take_errors() {
    e.into_diagnostic(&handler).emit();
  }

  let ts_module = parser.parse_typescript_module()
    .map_err(|e| e.into_diagnostic(&handler).emit())
    .expect("Failed to parse module.");

  for node in ts_module.body {
    let module_decl = node.as_module_decl();
    if let Some(m) = module_decl {
      if m.is_import() {
        let import = m.as_import().unwrap();
        println!("Hello from {}", import.src.value);
      }
    }
  }



  let c = swc::Compiler::new(cm.clone());

  let file_text: String = std::fs::read_to_string(&file_path).expect(&format!(
      "error opening source file \"{:?}\"",
      file_path
  ));

  let fm = cm.new_source_file(
    FileName::Custom(file_path.to_str().unwrap().into()), 
    file_text
  );

}


#[cfg(test)]
mod test {
  use crate::fence::{Fence, ParsedFence};
  use crate::walk_dirs::{discover_fences_and_files, SourceFile, WalkFileData};
  use find_ts_imports::SourceFileImportData;
  use std::collections::HashSet;
  use std::iter::{FromIterator, Iterator};
use std::path::PathBuf;

use super::print_imports_from_swc;



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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

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
  fn test_simple_contains_subsubdir_fence() {
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

    let expected_root_ts_file = SourceFile {
      source_file_path: "tests/walk_dir_simple/rootFile.ts".to_owned(),
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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

    let expected_subdir_ts_file = SourceFile {
      source_file_path: "tests/walk_dir_simple/subdir/subDirFile.ts".to_owned(),
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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

    let expected_subdir_ts_file = SourceFile {
      source_file_path: "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts".to_owned(),
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

  #[test]
  fn test_print_imports_from_swc() {
    let filename = "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts";
    print_imports_from_swc(&PathBuf::from(filename.to_owned()));
  }
}
