use std::path::Path;
use std::{path::PathBuf, sync::Arc};

use swc_core::common::comments::SingleThreadedComments;
use swc_core::common::errors::Handler;
use swc_core::common::{Globals, Mark, SourceMap, GLOBALS};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::visit::{Fold, VisitWith};
use swc_ecma_parser::{Capturing, Parser};

use swc_utils::create_lexer;

use crate::parse::exports_visitor::ExportsVisitor;
use crate::parse::RawImportExportInfo;

#[derive(Debug, thiserror::Error)]
pub enum SourceFileParseError {
    #[error("Auto-generated file")]
    AutogeneratedFile,
    #[error("Unable to load {0} file: {1}")]
    LoadFile(PathBuf, std::io::Error),
    #[error("TypeScript syntax error in {0}: {1}")]
    TypeScriptSyntax(PathBuf, String),
    #[error("Parser error in {0}: {1}")]
    Parser(PathBuf, String),
}

/// Gets the _unresolved_ import/export info from a file by reading it from disk and parsing it.
pub fn get_file_import_export_info(
    file_path: &Path,
) -> Result<RawImportExportInfo, SourceFileParseError> {
    let cm = Arc::<SourceMap>::default();
    let fm = match cm.load_file(file_path) {
        Ok(f) => f,
        Err(e) => return Err(SourceFileParseError::LoadFile(file_path.to_path_buf(), e)),
    };
    if fm.src.contains("// This file is auto-generated") {
        return Err(SourceFileParseError::AutogeneratedFile);
    }

    let dest_vector: Vec<u8> = Vec::new();
    let dst = Box::new(dest_vector);
    let handler = Handler::with_emitter_writer(dst, Some(cm.clone()));
    let comments = SingleThreadedComments::default();
    let lexer = create_lexer(&fm, Some(&comments));
    let capturing = Capturing::new(lexer);

    let mut parser = Parser::new_from(capturing);
    let errors = parser.take_errors();

    if !errors.is_empty() {
        return Err(SourceFileParseError::Parser(
            file_path.to_path_buf(),
            parser
                .take_errors()
                .into_iter()
                .map(|x| x.into_diagnostic(&handler).message())
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }

    // Parse file as typescript module to find parse errors.
    let ts_module = match parser.parse_typescript_module() {
        Ok(module) => module,
        Err(error) => {
            let mut diagnostic = error.into_diagnostic(&handler);
            // Avoid panic
            diagnostic.cancel();
            return Err(SourceFileParseError::TypeScriptSyntax(
                file_path.to_path_buf(),
                diagnostic.message(),
            ));
        }
    };

    let mut visitor = ExportsVisitor::new(comments);

    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        // Create resolver for variables
        let mut resolver = resolver(Mark::fresh(Mark::root()), Mark::fresh(Mark::root()), true);
        // Assign tags to identifiers
        let resolved = resolver.fold_module(ts_module.clone());
        // Do ast walk with our visitor
        resolved.visit_with(&mut visitor)
    });

    Ok(RawImportExportInfo {
        imported_path_ids: visitor.imported_ids_path_name,
        require_paths: visitor.require_paths,
        imported_paths: visitor.imported_paths,
        export_from_ids: visitor.export_from_ids, // TODO replace with ExportVisitor maps
        exported_ids: visitor.exported_ids,
        executed_paths: visitor.executed_paths,
    })
}
