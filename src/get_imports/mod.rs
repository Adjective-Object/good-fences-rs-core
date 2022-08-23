use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swc_common::errors::Handler;
use swc_common::SourceFile;
use swc_core::visit::visit_module;
use swc_ecma_parser::Capturing;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
mod require_import_visitor;
mod utils;

pub use require_import_visitor::*;
use utils::get_import_specifier_name;

use crate::error::GetImportError;

pub fn get_imports_map_from_file<'a>(
    file_path: &'a PathBuf,
) -> Result<HashMap<String, Option<HashSet<String>>>, GetImportError> {
    get_imports_from_file(&file_path)
}

fn get_imports_from_file<'a>(
    file_path: &'a PathBuf,
) -> Result<HashMap<String, Option<HashSet<String>>>, GetImportError> {
    let path_string = match file_path.to_str() {
        Some(path) => path,
        None => {
            return Err(GetImportError::PathError {
                filepath: file_path.clone(),
            })
        }
    };
    let cm = Arc::<swc_common::SourceMap>::default();
    let fm = match cm.load_file(Path::new(path_string)) {
        Ok(f) => f,
        Err(e) => {
            return Err(GetImportError::FileDoesNotExist {
                filepath: path_string.to_string(),
                io_errors: vec![e],
            });
        }
    };

    let mut parser_errors: Vec<String> = Vec::new();
    let dest_vector: Vec<u8> = Vec::new();
    let dst = Box::new(dest_vector);
    let handler = Handler::with_emitter_writer(dst, Some(cm.clone()));
    let lexer = create_lexer(&fm);
    let capturing = Capturing::new(lexer);
    let mut parser = Parser::new_from(capturing);

    let errors = parser.take_errors();

    if !errors.is_empty() {
        for error in errors {
            let mut diagnostic = error.into_diagnostic(&handler);
            parser_errors.push(diagnostic.message());
            diagnostic.cancel();
        }
        return Err(GetImportError::ParseTsFileError {
            filepath: path_string.to_string(),
            parser_errors,
        });
    }

    let ts_module = match parser.parse_typescript_module() {
        Ok(module) => module,
        Err(error) => {
            let mut diagnostic = error.into_diagnostic(&handler);
            parser_errors.push(diagnostic.message());
            diagnostic.cancel();
            return Err(GetImportError::ParseTsFileError {
                filepath: path_string.to_string(),
                parser_errors,
            });
        }
    };

    let mut visitor = ImportPathCheckerVisitor::new();

    visit_module(&mut visitor, &ts_module);

    let mut import_paths = HashMap::from(visitor.imports_map);

    let imports_map = capture_imports_map(ts_module, fm);

    return Ok(imports_map);
}

fn capture_imports_map(
    ts_module: swc_ecma_ast::Module,
    fm: Arc<SourceFile>,
) -> HashMap<String, Option<HashSet<String>>> {
    let mut imports_map: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    ts_module.body.iter().for_each(|node| {
        if node.is_module_decl() {
            if let Some(module_decl) = node.as_module_decl() {
                if module_decl.is_import() {
                    // let source_text = &fm.src.as_bytes().to_vec();
                    let i = module_decl.as_import().unwrap(); // Safe to unwrap due to previous is_import assertion
                    let specs_name: Vec<String> = i
                        .specifiers
                        .iter()
                        .filter_map(|spec| -> Option<String> {
                            return get_import_specifier_name(&fm, spec);
                        })
                        .collect();
                    let import_source = i.src.value.to_string();
                    match imports_map.get(&import_source) {
                        Some(Some(current_set)) => {
                            // TODO find a way to append data to current set instead of copying it into an new one
                            let mut new_set: HashSet<String> =
                                HashSet::from_iter(current_set.iter().map(|v| v).cloned());
                            for val in specs_name {
                                new_set.insert(val);
                            }
                            imports_map.insert(import_source, Some(new_set));
                        }
                        _ => {
                            if specs_name.is_empty() {
                                imports_map.insert(import_source, None);
                            } else {
                                let new_set: HashSet<String> =
                                    HashSet::from_iter(specs_name.iter().cloned());
                                imports_map.insert(import_source, Some(new_set));
                            }
                        }
                    }
                }
            }
        }
    });
    imports_map
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
    use crate::get_imports::{get_imports_from_file, get_imports_map_from_file};
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    #[test]
    fn test_get_imports_from_file() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let imports = get_imports_from_file(&PathBuf::from(filename.to_owned())).unwrap();
        assert_eq!(3, imports.len());
    }

    #[test]
    fn test_get_imports_map() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let import_map = get_imports_from_file(&PathBuf::from(filename.to_owned())).unwrap();
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
        let filename = "path/to/nowhere/nothing.ts";
        let imports = get_imports_map_from_file(&PathBuf::from(filename.to_owned())).map_err(|e| e);
        assert!(imports.is_err());
        assert_eq!("IO Errors found while trying to parse path/to/nowhere/nothing.ts : [Os { code: 3, kind: NotFound, message: \"The system cannot find the path specified.\" }]".to_string(), imports.unwrap_err().to_string());
    }

    #[test]
    fn test_parser_error() {
        let filename = "tests/good_fences_integration/src/parseError/parseError.ts";
        let imports = get_imports_from_file(&PathBuf::from(filename.to_owned()));
        assert!(imports.is_err());
        let error = imports.unwrap_err();
        assert_eq!(
            "Error parsing tests/good_fences_integration/src/parseError/parseError.ts :\n Expected ';', '}' or <eof>".to_string(),
            error.to_string()
        );
    }
}
