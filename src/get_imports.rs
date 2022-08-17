use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swc_common::errors::{ColorConfig, Handler};
use swc_common::source_map::Pos;
use swc_ecma_ast::Str;
use swc_ecma_parser::Capturing;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};

use crate::error::GetImportError;
struct SourceSpecifiers {
    specifiers: Vec<swc_ecma_ast::ImportSpecifier>,
    source: Str,
}

pub fn get_imports_map_from_file<'a>(
    file_path: &'a PathBuf,
) -> Result<HashMap<String, Option<HashSet<String>>>, GetImportError> {
    if file_path.exists() {
        let imports = match get_imports_from_file(&file_path) {
            Ok(i) => i,
            Err(e) => return Err(e),
        };
        return get_imports_map(&imports, &file_path);
    }
    Err(GetImportError::FileDoesNotExist(file_path.to_str().unwrap().to_string()))
}

fn get_imports_from_file<'a>(file_path: &'a PathBuf) -> Result<Vec<SourceSpecifiers>, GetImportError> {
    let path_string = match file_path.to_str() {
        Some(path) => path,
        None => return Err(GetImportError::ReadTsFileError(None)),
    };
    let cm = Arc::<swc_common::SourceMap>::default();
    let fm = match cm.load_file(Path::new(path_string)) {
        Ok(f) => f,
        Err(_) => return Err(GetImportError::ReadTsFileError(Some(path_string.to_string()))),
    };
    let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));

    let lexer = create_lexer(&fm);

    let capturing = Capturing::new(lexer);

    let mut parser = Parser::new_from(capturing);

    for e in parser.take_errors() {
        e.into_diagnostic(&handler).emit();
    }

    let ts_module = match parser
        .parse_typescript_module()
        .map_err(|e| e.into_diagnostic(&handler).emit())
        {
            Ok(module) => module,
            Err(_) => return Err(GetImportError::ParseTsFileError(path_string.to_string())),
        };

    let imports: Vec<_> = ts_module
        .body
        .iter()
        .filter_map(|node| -> Option<SourceSpecifiers> {
            if node.is_module_decl() {
                if let Some(module_decl) = node.as_module_decl() {
                    if module_decl.is_import() {
                        let i = module_decl.as_import().unwrap(); // Safe to unwrap due to previous is_import assertion
                        return Some(SourceSpecifiers {
                            specifiers: i.specifiers.to_vec(),
                            source: i.src.clone(),
                        });
                    }
                }
            }
            None
        })
        .collect();

    return Ok(imports);
}

fn get_string_of_span<'a>(file_text: &'a Vec<u8>, span: &'a swc_common::Span) -> String {
    String::from_utf8_lossy(&file_text[span.lo().to_usize() - 1..span.hi().to_usize() - 1])
        .to_string()
}

fn get_imports_map(
    imports: &Vec<SourceSpecifiers>,
    importer_file_path: &PathBuf,
) -> Result<HashMap<String, Option<HashSet<String>>>, GetImportError> {
    let mut imports_map: HashMap<String, Option<HashSet<String>>> = HashMap::new();

    imports.iter().for_each(|import| {
        let set: HashSet<String> = import
            .specifiers
            .iter()
            .filter(|spec| spec.is_default() || spec.is_named())
            .filter_map(|spec| -> Option<String> {
                let file_text = match std::fs::read(importer_file_path) {
                    Ok(text) => text,
                    _ => {
                        return None;
                    }
                };
                if let Some(default) = spec.as_default() {
                    return Some(get_string_of_span(&file_text, &default.span));
                }
                if let Some(named) = spec.as_named() {
                    return Some(get_string_of_span(&file_text, &named.span));
                }
                None
            })
            .collect();
        match imports_map.get(&import.source.value.to_string()) {
            Some(Some(current_set)) => {
                // TODO find a way to append data to current set instead of copying it into an new one
                let mut new_set: HashSet<String> =
                    HashSet::from_iter(current_set.iter().map(|v| v).cloned());
                for val in set {
                    new_set.insert(val);
                }
                imports_map.insert(import.source.value.to_string(), Some(new_set));
            }
            _ => {
                if set.is_empty() {
                    imports_map.insert(import.source.value.to_string(), None);
                } else {
                    imports_map.insert(import.source.value.to_string(), Some(set));
                }
            }
        }
    });
    Ok(imports_map)
}

fn create_lexer<'a>(fm: &'a swc_common::SourceFile) -> Lexer<'a, StringInput<'a>> {
    let lexer = Lexer::new(
        Syntax::Typescript(Default::default()),
        Default::default(),
        StringInput::from(fm),
        None,
    );
    lexer
}

#[cfg(test)]
mod test {
    use crate::{get_imports::{get_imports_from_file, get_imports_map, get_imports_map_from_file}};
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    #[test]
    fn test_get_imports_from_file() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let imports = get_imports_from_file(&PathBuf::from(filename.to_owned())).unwrap();
        assert_eq!(4, imports.len());
    }

    #[test]
    fn test_get_imports_map() {
        // TODO consider multiple imports from same file in ts files
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        // let mut expected_imports = map!["./helperA1" => Some(set!(""))];
        let source_specs = get_imports_from_file(&PathBuf::from(filename.to_owned())).unwrap();
        let import_map = get_imports_map(&source_specs, &PathBuf::from(filename.to_owned())).unwrap();
        let expected_map: HashMap<String, Option<HashSet<String>>> = HashMap::from([
            (
                String::from("../componentB/componentB"),
                Some(HashSet::from(["componentB".to_string()])),
            ),
            (
                String::from("./helperA1"),
                Some(HashSet::from([
                    "helperA1".to_string(),
                    "some".to_string(),
                    "other".to_string(),
                    "stuff".to_string(),
                ])),
            ),
            (
                String::from("./helperA2"),
                Some(HashSet::from(["helperA2".to_string()])),
            ),
        ]);
        assert_eq!(import_map, expected_map);
    }

    #[test]
    fn test_get_imports_from_non_existent_path() {
        // TODO consider multiple imports from same file in ts files
        let filename = "path/to/nowhere/nothing.ts";
        let source_specs = get_imports_map_from_file(&PathBuf::from(filename.to_owned())).map_err(|e| e);
        assert!(source_specs.is_err());
        match source_specs.unwrap_err() {
            crate::error::GetImportError::ParseTsFileError(_) => assert!(false),
            crate::error::GetImportError::ReadingImportError(_, _) => assert!(false),
            crate::error::GetImportError::ReadTsFileError(_) => assert!(false),
            crate::error::GetImportError::FileDoesNotExist(_) => assert!(true),
        }
        // assert!(source_specs.is_err());
    }
}
