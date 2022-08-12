use crate::fence::{parse_fence_file, Fence};
use jwalk::{WalkDirGeneric, Error};
use relative_path::RelativePath;
use serde::Deserialize;
use swc_common::source_map::Pos;
use swc_common::sync::Lrc;
use swc_common::{
    errors::{ColorConfig, Handler},
    FileName, FilePathMapping, SourceMap
};
use swc_ecma_ast::Str;
use swc_ecma_parser::Capturing;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
use std::collections::{HashSet, HashMap};
use std::convert::TryInto;
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
  pub imports: HashMap<String, Option<HashSet<String>>>,
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

                  let imports = get_imports_from_file(&file_path);
                  let imports_map = get_imports_map(&imports, &file_path);
                 
                  dir_entry.client_state = WalkFileData::SourceFile(SourceFile {
                    source_file_path: source_file_path.unwrap().to_string(),
                    imports: imports_map,
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


fn get_imports_map(imports: &Vec<SourceSpecifiers>, file_path: &PathBuf) -> HashMap<String, Option<HashSet<String>>> {
    let mut imports_map : HashMap<String, Option<HashSet<String>>> = HashMap::new();
    // imports.iter().for_each(|import| {
    //   import.specifiers
    // });
    imports.iter().for_each(|import| {
    
      let mut set: HashSet<String> = import.specifiers.iter().filter_map(|spec| -> Option<String> {
        let file_text = std::fs::read(file_path).expect(&format!(
          "error opening source file \"{:?}\"",
          file_path
        ));
        if let Some(default) = spec.as_default() {
          let text =  &file_text[default.span.lo().to_usize()-1..default.span.hi().to_usize()-1];
          return Some(String::from_utf8_lossy(text).to_string());
        }
        if let Some(named) = spec.as_named() {
          
          let text =  &file_text[named.span.lo().to_usize()-1..named.span.hi().to_usize()-1];
          println!("{}", String::from_utf8_lossy(text));
          return Some(String::from_utf8_lossy(text).to_string());
        }
        None
      }).collect();

      if let Some(current_set) = imports_map.get(&import.source.value.to_string()) {
        if let  Some(current_set) = current_set {
          let mut new_set: HashSet<String> = HashSet::from_iter(current_set.iter().map(|v| v).cloned());
          for val in set {
            new_set.insert(val);
          }
          imports_map.insert(import.source.value.to_string(), Some(new_set.to_owned()));
        } else {

        }
      } else {
        if set.is_empty() {
          imports_map.insert(import.source.value.to_string(), None);
        } else {
          imports_map.insert(import.source.value.to_string(), Some(set));
        }
      }
    
    });
    imports_map
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

pub struct SourceSpecifiers {
  specifiers: Vec<swc_ecma_ast::ImportSpecifier>,
  source: Str
}

pub fn get_imports_from_file<'a>(file_path: &'a PathBuf) -> Vec<SourceSpecifiers>{
  let cm = Arc::<swc_common::SourceMap>::default();
  let fm = cm.load_file(Path::new(file_path.to_str().unwrap())).expect("Could not load file");
  let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
  
  let lexer = create_lexer(&fm);

  let capturing = Capturing::new(lexer);

  let mut parser = Parser::new_from(capturing);

  for e in parser.take_errors() {
    e.into_diagnostic(&handler).emit();
  }



  let ts_module = parser.parse_typescript_module()
    .map_err(|e| e.into_diagnostic(&handler).emit())
    .expect("Failed to parse module.");

  let imports: Vec<_> = ts_module.body.iter().filter_map(|node| -> Option<SourceSpecifiers> {
    if node.is_module_decl() {
      if let Some(module_decl) = node.as_module_decl() {
        if module_decl.is_import() {
          let i = node.as_module_decl().unwrap().as_import().unwrap();
          return Some(SourceSpecifiers {
            specifiers: i.specifiers.clone().to_vec(),
            source: i.src.clone()
          });
        }
      }
    }
    None
  }).collect();

  return imports;
}


#[cfg(test)]
mod test {
  use crate::fence::{Fence, ParsedFence};
  use crate::walk_dirs::{discover_fences_and_files, SourceFile, WalkFileData};
  use std::collections::HashSet;
  use std::iter::{FromIterator, Iterator};
use std::path::PathBuf;

use super::{get_imports_from_file, get_imports_map};



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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

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
    let discovered: Vec<WalkFileData> = discover_fences_and_files("tests/walk_dir_simple");

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
  fn tesT_get_imports_from_file() {
    let filename = "tests/walk_dir_simple/subdir/subsubdir/subSubDirFile.ts";
    get_imports_from_file(&PathBuf::from(filename.to_owned()));
  }

  #[test]
  fn test_get_imports_map() {
    // TODO consider multiple imports from same file in ts files
    let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
    // let mut expected_imports = map!["./helperA1" => Some(set!(""))];
    let source_specs = get_imports_from_file(&PathBuf::from(filename.to_owned()));
    let import_map = get_imports_map(&source_specs, &PathBuf::from(filename.to_owned()));
    import_map.iter().for_each(|f| {
      let (key, value) = f;
      if let Some(value) = value {
        println!("Key {}", key);
        print!("Values ");
        value.iter().for_each(|v| {
          print!(" {} ", v);
        });
        println!("");

      }
    });
  }
}
