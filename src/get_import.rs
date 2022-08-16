use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swc_common::errors::{ColorConfig, Handler};
use swc_common::source_map::Pos;
use swc_ecma_ast::Str;
use swc_ecma_parser::Capturing;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
pub struct SourceSpecifiers {
    specifiers: Vec<swc_ecma_ast::ImportSpecifier>,
    source: Str,
}

pub fn get_imports_map_from_file<'a>(file_path: &'a PathBuf) -> HashMap<String, Option<HashSet<String>>> {
    let imports = get_imports_from_file(&file_path);
    get_imports_map(&imports, &file_path)
}

fn get_imports_from_file<'a>(file_path: &'a PathBuf) -> Vec<SourceSpecifiers> {
    let cm = Arc::<swc_common::SourceMap>::default();
    let fm = cm
        .load_file(Path::new(file_path.to_str().unwrap()))
        .expect("Could not load file");
    let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));

    let lexer = create_lexer(&fm);

    let capturing = Capturing::new(lexer);

    let mut parser = Parser::new_from(capturing);

    for e in parser.take_errors() {
        e.into_diagnostic(&handler).emit();
    }

    let ts_module = parser
        .parse_typescript_module()
        .map_err(|e| e.into_diagnostic(&handler).emit())
        .expect("Failed to parse module.");

    let imports: Vec<_> = ts_module
        .body
        .iter()
        .filter_map(|node| -> Option<SourceSpecifiers> {
            if node.is_module_decl() {
                if let Some(module_decl) = node.as_module_decl() {
                    if module_decl.is_import() {
                        let i = node.as_module_decl().unwrap().as_import().unwrap();
                        return Some(SourceSpecifiers {
                            specifiers: i.specifiers.clone().to_vec(),
                            source: i.src.clone(),
                        });
                    }
                }
            }
            None
        })
        .collect();

    return imports;
}

fn get_string_of_span<'a>(file_text: &'a Vec<u8>, span: &'a swc_common::Span) -> String {
    String::from_utf8_lossy(&file_text[span.lo().to_usize() - 1..span.hi().to_usize() - 1])
        .to_string()
}

fn get_imports_map(
    imports: &Vec<SourceSpecifiers>,
    importer_file_path: &PathBuf,
) -> HashMap<String, Option<HashSet<String>>> {
    let mut imports_map: HashMap<String, Option<HashSet<String>>> = HashMap::new();

    imports.iter().for_each(|import| {
        let set: HashSet<String> = import
            .specifiers
            .iter()
            .filter_map(|spec| -> Option<String> {
                let file_text =std::fs::read(importer_file_path).expect(&format!("error opening source file \"{:?}\"", importer_file_path));
                if let Some(default) = spec.as_default() {
                    let text = get_string_of_span(&file_text, &default.span);
                    return Some(text);
                }
                if let Some(named) = spec.as_named() {
                    let text = get_string_of_span(&file_text, &named.span);
                    return Some(text);
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
    use crate::get_import::{get_imports_from_file, get_imports_map};
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
    };

    #[test]
    fn test_get_imports_from_file() {
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        let imports = get_imports_from_file(&PathBuf::from(filename.to_owned()));
        assert_eq!(4, imports.len());
    }

    #[test]
    fn test_get_imports_map() {
        // TODO consider multiple imports from same file in ts files
        let filename = "tests/good_fences_integration/src/componentA/componentA.ts";
        // let mut expected_imports = map!["./helperA1" => Some(set!(""))];
        let source_specs = get_imports_from_file(&PathBuf::from(filename.to_owned()));
        let import_map = get_imports_map(&source_specs, &PathBuf::from(filename.to_owned()));
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
}
