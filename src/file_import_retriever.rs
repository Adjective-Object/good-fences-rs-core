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
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
  
pub fn get_imports_map(imports: &Vec<SourceSpecifiers>, file_path: &PathBuf) -> HashMap<String, Option<HashSet<String>>> {
    let mut imports_map : HashMap<String, Option<HashSet<String>>> = HashMap::new();
    // imports.iter().for_each(|import| {
    //   import.specifiers
    // });
    imports.iter().for_each(|import| {
    
      let set: HashSet<String> = import.specifiers.iter().filter_map(|spec| -> Option<String> {
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

#[cfg(test)]
mod test {
    use crate::file_import_retriever::{*};
    #[test]
    fn test_get_imports_from_file() {
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