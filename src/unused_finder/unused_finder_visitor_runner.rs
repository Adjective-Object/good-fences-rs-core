use std::collections::{HashMap, HashSet};
use std::{path::PathBuf, sync::Arc};

use swc_common::{errors::Handler, Globals, Mark, GLOBALS};
use swc_ecma_parser::{Capturing, Parser};
use swc_ecmascript::transforms::resolver;
use swc_ecmascript::visit::{fold_module, visit_module};

use crate::get_imports::create_lexer;

use super::node_visitor::{ExportedItem, ImportedItem, UnusedFinderVisitor};

#[derive(Debug, PartialEq, Eq)]
pub struct ImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_path_ids: HashMap<String, HashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: HashSet<String>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: HashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: HashMap<String, HashSet<ExportedItem>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: HashSet<ExportedItem>,
}

impl ImportExportInfo {
    pub fn new() -> Self {
        Self {
            imported_path_ids: HashMap::new(),
            require_paths: HashSet::new(),
            imported_paths: HashSet::new(),
            export_from_ids: HashMap::new(),
            exported_ids: HashSet::new(),
        }
    }
}

impl Default for ImportExportInfo {
    fn default() -> Self {
        Self::new()
    }
}

pub fn get_import_export_paths_map(file_path: String) -> Result<ImportExportInfo, String> {
    let path = PathBuf::from(&file_path);

    let cm = Arc::<swc_common::SourceMap>::default();
    let fm = match cm.load_file(path.as_path()) {
        Ok(f) => f,
        Err(_) => todo!(), // TODO create err module
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
        todo!() // TODO Create err module
    }

    // Parse file as typescript module to find parse errors.
    let ts_module = match parser.parse_typescript_module() {
        Ok(module) => module,
        Err(error) => {
            let mut diagnostic = error.into_diagnostic(&handler);
            // Push error to vec of errors
            parser_errors.push(diagnostic.message());
            // Avoid panic
            diagnostic.cancel();
            todo!();
        }
    };

    let mut visitor = UnusedFinderVisitor::new();

    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        // Create resolver for variables
        let mut resolver = resolver(Mark::fresh(Mark::root()), Mark::fresh(Mark::root()), true);
        // Assign tags to identifiers
        let resolved = fold_module(&mut resolver, ts_module.clone());
        // Do ast walk with our visitor
        visit_module(&mut visitor, &resolved);
    });

    Ok(ImportExportInfo {
        imported_path_ids: visitor.imported_ids_path_name,
        require_paths: visitor.require_paths,
        imported_paths: visitor.imported_paths,
        export_from_ids: visitor.export_from_ids, // TODO replace with ExportVisitor maps
        exported_ids: visitor.exported_ids,
    })
}
