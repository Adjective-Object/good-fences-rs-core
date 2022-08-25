use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swc_common::{Globals, GLOBALS, Mark};
use swc_common::errors::Handler;
use swc_core::visit::{visit_module, fold_module};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
use swc_ecma_parser::{Capturing, TsConfig};
mod import_path_visitor;
mod utils;

pub use import_path_visitor::*;

use crate::error::GetImportError;

pub fn get_imports_map_from_file<'a>(
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

    let mut visitor = ImportPathVisitor::new();

    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        let mut resolver = swc_core::transforms::resolver(
            Mark::fresh(Mark::root()),
            Mark::fresh(Mark::root()),
            true,
        );
        let resolved = fold_module(&mut resolver, ts_module.clone());
        visit_module(&mut visitor, &resolved);
    });
    let imports_map = get_imports_map_from_visitor(visitor);

    return Ok(imports_map);
}

fn get_imports_map_from_visitor(
    visitor: ImportPathVisitor,
) -> HashMap<String, Option<HashSet<String>>> {
    let mut final_imports_map: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    visitor
        .imports_map
        .iter()
        .for_each(|(k, v)| match final_imports_map.get_mut(k) {
            Some(Some(specifiers)) => {
                for spec in v {
                    specifiers.insert(spec.clone());
                }
            }
            Some(None) | None => {
                if !v.is_empty() {
                    final_imports_map.insert(k.clone(), Some(v.clone()));
                }
            }
        });
    visitor.import_paths.iter().for_each(|path| {
        if !final_imports_map.contains_key(path) {
            final_imports_map.insert(path.clone(), None);
        }
    });
    visitor.require_paths.iter().for_each(|path| {
        if !final_imports_map.contains_key(path) {
            final_imports_map.insert(path.clone(), None);
        }
    });
    final_imports_map
}

fn create_lexer<'a>(fm: &'a swc_common::SourceFile) -> Lexer<'a, StringInput<'a>> {
    let lexer = Lexer::new(
        Syntax::Typescript(TsConfig {
            tsx: true,
            ..Default::default()
        }),
        Default::default(),
        StringInput::from(fm),
        None,
    );
    lexer
}

#[cfg(test)]
mod test {
    use crate::get_imports::get_imports_map_from_file;
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    #[test]
    fn test_get_imports_from_file() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let imports = get_imports_map_from_file(&PathBuf::from(filename.to_owned())).unwrap();
        assert_eq!(3, imports.len());
    }

    #[test]
    fn test_get_imports_map() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let import_map = get_imports_map_from_file(&PathBuf::from(filename.to_owned())).unwrap();
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
        let imports = get_imports_map_from_file(&PathBuf::from(filename.to_owned()));
        assert!(imports.is_err());
        let error = imports.unwrap_err();
        assert_eq!(
            "Error parsing tests/good_fences_integration/src/parseError/parseError.ts :\n Expected ';', '}' or <eof>".to_string(),
            error.to_string()
        );
    }
}
