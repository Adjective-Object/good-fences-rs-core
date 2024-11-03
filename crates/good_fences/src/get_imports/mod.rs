use std::collections::{HashMap, HashSet};
use std::path::Path;
use swc_common::comments::Comments;
use swc_common::errors::Handler;
use swc_common::sync::Lrc;
use swc_common::{Globals, Mark, GLOBALS};
use swc_common::{SourceFile, SourceMap};
use swc_ecma_parser::{lexer::Lexer, StringInput, Syntax};
use swc_ecma_parser::{Capturing, Parser, TsSyntax};
use swc_ecma_transforms::resolver;
use swc_ecma_visit::{FoldWith, VisitWith};
mod import_path_visitor;
use crate::error::GetImportError;

pub use import_path_visitor::*;

pub type FileImports = HashMap<String, Option<HashSet<String>>>;

pub fn get_imports_map_from_file<P: AsRef<str>>(
    file_path: &P,
) -> Result<FileImports, GetImportError> {
    let path_string: &str = file_path.as_ref();
    let cm = Lrc::<SourceMap>::default();
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
    let lexer = create_lexer(&fm, None);
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
        let mut resolver = resolver(Mark::fresh(Mark::root()), Mark::fresh(Mark::root()), true);
        let resolved = ts_module.clone().fold_with(&mut resolver);
        resolved.visit_with(&mut visitor);
    });
    let imports_map = get_imports_map_from_visitor(visitor);

    Ok(imports_map)
}

fn get_imports_map_from_visitor(visitor: ImportPathVisitor) -> FileImports {
    let mut final_imports_map: FileImports = HashMap::new();
    let ImportPathVisitor {
        mut require_paths,
        mut import_paths,
        mut imports_map,
        ..
    } = visitor;

    require_paths.drain().for_each(|path| {
        final_imports_map.insert(path, None);
    });

    import_paths.drain().for_each(|path| {
        final_imports_map.insert(path, None);
    });

    imports_map
        .drain()
        .for_each(|(k, v)| match final_imports_map.get_mut(&k) {
            Some(Some(specifiers)) => {
                for spec in v {
                    specifiers.insert(spec);
                }
            }
            Some(None) | None => {
                if !v.is_empty() {
                    final_imports_map.insert(k, Some(v));
                }
            }
        });

    final_imports_map
}

pub fn create_lexer<'a>(fm: &'a SourceFile, comments: Option<&'a dyn Comments>) -> Lexer<'a> {
    let filename = fm.name.to_string();
    let lexer = Lexer::new(
        Syntax::Typescript(TsSyntax {
            tsx: filename.ends_with(".tsx") || filename.ends_with(".jsx"),
            decorators: true,
            ..Default::default()
        }),
        Default::default(),
        StringInput::from(fm),
        comments,
    );
    lexer
}

#[cfg(test)]
mod test {
    use crate::get_imports::{get_imports_map_from_file, FileImports};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_get_imports_from_file() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let imports = get_imports_map_from_file(&filename).unwrap();
        assert_eq!(3, imports.len());
    }

    #[test]
    fn test_get_imports_map() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let import_map = get_imports_map_from_file(&filename).unwrap();
        let expected_map: FileImports = HashMap::from([
            (
                String::from("../componentB/componentB"),
                Some(HashSet::from(["default".to_string()])),
            ),
            (
                String::from("./helperA1"),
                Some(HashSet::from([
                    "default".to_string(),
                    "some".to_string(),
                    "other".to_string(),
                    "stuff".to_string(),
                ])),
            ),
            (
                String::from("./helperA2"),
                Some(HashSet::from(["default".to_string()])),
            ),
        ]);
        assert_eq!(import_map, expected_map);
    }

    #[test]
    fn test_get_imports_from_non_existent_path() {
        let filename = "path/to/nowhere/nothing.ts";
        let imports = get_imports_map_from_file(&filename);
        assert!(imports.is_err());
        #[cfg(target_os = "windows")]
        assert_eq!("IO Errors found while trying to parse path/to/nowhere/nothing.ts : [Os { code: 3, kind: NotFound, message: \"The system cannot find the path specified.\" }]".to_string(), imports.unwrap_err().to_string());

        #[cfg(not(target_os = "windows"))]
        assert_eq!("IO Errors found while trying to parse path/to/nowhere/nothing.ts : [Os { code: 2, kind: NotFound, message: \"No such file or directory\" }]".to_string(), imports.unwrap_err().to_string());
    }

    #[test]
    fn test_parser_error() {
        let filename = "tests/good_fences_integration/src/parseError/parseError.ts";
        let imports = get_imports_map_from_file(&filename);
        assert!(imports.is_err());
        let error = imports.unwrap_err();
        assert_eq!(
            "Error parsing tests/good_fences_integration/src/parseError/parseError.ts :\n Expected ';', '}' or <eof>".to_string(),
            error.to_string()
        );
    }
}
