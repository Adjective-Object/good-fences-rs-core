use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_semantic::SemanticBuilder;

mod import_path_visitor;
use crate::error::GetImportError;

pub use import_path_visitor::*;

pub type FileImports = HashMap<String, Option<HashSet<String>>>;

// Per-worker-thread arena reused across `get_imports_map_from_file` calls.
thread_local! {
    static PARSE_ARENA: RefCell<Allocator> = RefCell::new(Allocator::default());
}

pub fn get_imports_map_from_file<P: AsRef<str>>(
    file_path: &P,
) -> Result<FileImports, GetImportError> {
    let path = Path::new(file_path.as_ref());
    let source = std::fs::read_to_string(path).map_err(|e| GetImportError::FileDoesNotExist {
        filepath: path.display().to_string(),
        io_errors: vec![e],
    })?;

    PARSE_ARENA.with(|arena_cell| {
        let arena = arena_cell.borrow();
        let ret = oxc_utils_parse::parse_file(&arena, &source, path);

        if ret.panicked || !ret.errors.is_empty() {
            let parser_errors = ret.errors.iter().map(|e| e.message.to_string()).collect();
            drop(ret);
            drop(arena);
            arena_cell.borrow_mut().reset();
            return Err(GetImportError::ParseTsFileError {
                filepath: path.display().to_string(),
                parser_errors,
            });
        }

        let semantic = SemanticBuilder::new().build(&ret.program).semantic;
        let mut visitor = ImportPathVisitor::new(&semantic);
        visitor.visit_program(&ret.program);
        let imports = get_imports_map_from_visitor(visitor);

        drop(semantic);
        drop(ret);
        drop(arena);
        arena_cell.borrow_mut().reset();

        Ok(imports)
    })
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
        let err_str = imports.unwrap_err().to_string();
        assert!(
            err_str.starts_with("Error parsing tests/good_fences_integration/src/parseError/parseError.ts"),
            "unexpected error message: {err_str}"
        );
    }
}
