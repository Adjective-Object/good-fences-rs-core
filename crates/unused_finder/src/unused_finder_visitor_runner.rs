use std::{path::PathBuf, sync::Arc};

use swc_core::common::comments::SingleThreadedComments;
use swc_core::common::errors::Handler;
use swc_core::common::{Globals, Mark, SourceMap, GLOBALS};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::visit::{Fold, VisitWith};
use swc_ecma_parser::{Capturing, Parser};

use anyhow;
use swc_utils::create_lexer;

use crate::parse::exports_visitor::ExportsVisitor;
use crate::parse::{ExportedItem, FileImportExportInfo};

#[derive(Debug, thiserror::Error)]
pub enum SourceFileParseError {
    #[error("Auto-generated file")]
    AutogeneratedFileError,
    #[error("Unable to load {0} file")]
    LoadFileError(String),
    #[error("TypeScript syntax error: {0}")]
    TypeScriptSyntaxError(String),
    #[error("Parser error: {0}")]
    ParserError(String),
}

pub fn get_import_export_paths_map(
    file_path: String,
    skipped_items: Arc<Vec<regex::Regex>>,
) -> anyhow::Result<FileImportExportInfo> {
    let path = PathBuf::from(&file_path);

    let cm = Arc::<SourceMap>::default();
    let fm = match cm.load_file(path.as_path()) {
        Ok(f) => f,
        Err(e) => {
            return Err(anyhow!(SourceFileParseError::LoadFileError(file_path.clone())).context(e))
        }
    };
    if fm.src.contains("// This file is auto-generated") {
        return Err(SourceFileParseError::AutogeneratedFileError.into());
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
        let mut error = anyhow::Error::new(SourceFileParseError::ParserError(file_path.clone()));
        for e in errors {
            let mut diagnostic = e.into_diagnostic(&handler);
            error = error.context(diagnostic.message());
            diagnostic.cancel();
        }
        return Err(error);
    }

    // Parse file as typescript module to find parse errors.
    let ts_module = match parser.parse_typescript_module() {
        Ok(module) => module,
        Err(error) => {
            let mut diagnostic = error.into_diagnostic(&handler);
            // Avoid panic
            diagnostic.cancel();
            return Err(
                anyhow::Error::new(SourceFileParseError::TypeScriptSyntaxError(
                    file_path.clone(),
                ))
                .context(diagnostic.message()),
            );
        }
    };

    let mut visitor = ExportsVisitor::new(skipped_items, comments);

    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        // Create resolver for variables
        let mut resolver = resolver(Mark::fresh(Mark::root()), Mark::fresh(Mark::root()), true);
        // Assign tags to identifiers
        let resolved = resolver.fold_module(ts_module.clone());
        // Do ast walk with our visitor
        resolved.visit_with(&mut visitor)
    });

    Ok(FileImportExportInfo {
        imported_path_ids: visitor.imported_ids_path_name,
        require_paths: visitor.require_paths,
        imported_paths: visitor.imported_paths,
        export_from_ids: visitor.export_from_ids, // TODO replace with ExportVisitor maps
        exported_ids: visitor
            .exported_ids
            .drain()
            .map(|metadata| ExportedItem {
                metadata,
                source_file_path: path.clone(),
            })
            .collect(),
        executed_paths: visitor.executed_paths,
    })
}
