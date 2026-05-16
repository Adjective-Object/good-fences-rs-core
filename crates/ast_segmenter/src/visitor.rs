use logger_srcfile::SrcFileLogger;
use swc_common::comments::SingleThreadedComments;
use swc_common::Spanned;
use swc_ecma_visit::VisitWith;

use crate::{
    raw_module_deps::RawModuleDeps,
    segment_info::RawSegment,
    visitors::{import_export_statement::ExportsVisitor, import_require_expr},
};

const RETURN: &str = "return";
const BREAK: &str = "break";
const CONTINUE: &str = "continue";

#[derive(thiserror::Error, Debug)]
pub enum StatementToSegmentError {
    #[error("\"{}\" statements should not occur at the module level", .0)]
    StatementUnexpectedInModuleScope(&'static str),
    #[error("With statements are not supported because they create new non-lexical names")]
    WithStatmentUnsupported,
}

fn module_item_to_segment(
    file_logger: &impl SrcFileLogger,
    comments: &SingleThreadedComments,
    module_item: &swc_ecma_ast::ModuleItem,
) -> Option<RawSegment> {
    match module_item {
        swc_ecma_ast::ModuleItem::Stmt(stmt) => {
            match stmt {
                swc_ecma_ast::Stmt::Decl(_)
                | swc_ecma_ast::Stmt::Expr(_)
                | swc_ecma_ast::Stmt::Block(_)
                | swc_ecma_ast::Stmt::Empty(_)
                | swc_ecma_ast::Stmt::Debugger(_)
                | swc_ecma_ast::Stmt::Labeled(_)
                | swc_ecma_ast::Stmt::Switch(_)
                | swc_ecma_ast::Stmt::If(_)
                | swc_ecma_ast::Stmt::Throw(_)
                | swc_ecma_ast::Stmt::Try(_)
                | swc_ecma_ast::Stmt::While(_)
                | swc_ecma_ast::Stmt::DoWhile(_)
                | swc_ecma_ast::Stmt::For(_)
                | swc_ecma_ast::Stmt::ForIn(_)
                | swc_ecma_ast::Stmt::ForOf(_) => {
                    let variables = ast_name_tracker::visitor::find_names(file_logger, stmt);
                    let imports_and_requires = import_require_expr::find_imports_and_requires(stmt);

                    // Convert dynamic imports and requires into RawModuleDeps
                    let dynamic_imports = imports_and_requires
                        .imported_paths
                        .names()
                        .into_iter()
                        .collect();
                    let requires = imports_and_requires
                        .require_paths
                        .names()
                        .into_keys()
                        .collect();

                    let module_deps = RawModuleDeps {
                        dynamic_imports,
                        requires,
                        ..Default::default()
                    };

                    Some(RawSegment {
                        module_deps,
                        variables,
                    })
                }
                swc_ecma_ast::Stmt::With(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::WithStatmentUnsupported,
                    );
                    None
                }
                swc_ecma_ast::Stmt::Return(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(RETURN),
                    );
                    None
                }
                swc_ecma_ast::Stmt::Break(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(BREAK),
                    );
                    None
                }
                swc_ecma_ast::Stmt::Continue(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(CONTINUE),
                    );
                    None
                }
            }
        }
        swc_ecma_ast::ModuleItem::ModuleDecl(module_decl) => {
            let variables = ast_name_tracker::visitor::find_names(file_logger, module_decl);

            // Run ExportsVisitor on the declaration to extract import/export deps
            let mut exports_visitor = ExportsVisitor::new(file_logger, comments);
            module_decl.visit_with(&mut exports_visitor);
            let module_deps: RawModuleDeps = exports_visitor.into();

            Some(RawSegment {
                module_deps,
                variables,
            })
        }
    }
}

/// Segment a parsed module into a list of `RawSegment`s — one per top-level
/// `ModuleItem`. Each segment combines variable scope analysis with module
/// dependency extraction.
pub fn segment_file(
    logger: &impl SrcFileLogger,
    module: &swc_ecma_ast::Module,
    comments: &SingleThreadedComments,
) -> Vec<RawSegment> {
    module
        .body
        .iter()
        .filter_map(|item| module_item_to_segment(logger, comments, item))
        .collect()
}

// represents a 2d dependency map between a set of IDs
struct Dependencies2D {
    size: u32,
    backing_bitmap: roaring::RoaringBitmap,
}
impl Dependencies2D {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            backing_bitmap: roaring::RoaringBitmap::new(),
        }
    }

    pub fn add_dependency(&mut self, from: u32, to: u32) {
        self.backing_bitmap.insert(self.idx(from, to));
    }

    pub fn depends_on(&self, from: u32, to: u32) -> bool {
        self.backing_bitmap.contains(self.idx(from, to))
    }

    fn idx(&self, from: u32, to: u32) -> u32 {
        from * self.size + to
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::raw_module_deps::{Symbol, SymbolTags, TaggedSymbol};
    use crate::{ExportedSymbol, ImportTarget, ReExportedSymbol};

    /// Parse source and return the segments produced by `segment_file`.
    fn segment(src: &str) -> Vec<RawSegment> {
        let cm = swc_common::sync::Lrc::<swc_common::SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            swc_common::sync::Lrc::new(swc_common::FileName::Custom("test.ts".into())),
            src.to_string(),
        );
        let lexer = swc_utils_parse::create_lexer(&fm, Some(&comments));
        let capturing = swc_ecma_parser::Capturing::new(lexer);
        let mut parser = swc_ecma_parser::Parser::new_from(capturing);
        let module = parser.parse_typescript_module().expect("parse failed");

        let logger = logger::StdioLogger::new();
        let file_logger = logger_srcfile::WrapFileLogger::new(cm, &logger);
        segment_file(&file_logger, &module, &comments)
    }

    // ── Mixed imports / exports / statements ───────────────────────────

    #[test]
    fn mixed_file_segment_count() {
        let segments = segment(
            r#"
            import { foo } from './foo';
            const x = foo();
            export function bar() { return x; }
            export default 42;
            "#,
        );
        // 4 top-level items → 4 segments
        assert_eq!(segments.len(), 4);
    }

    #[test]
    fn import_segment_has_correct_deps() {
        let segments = segment("import { foo, bar } from './foo';");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg.module_deps.imports.contains_key("./foo"));
        let symbols = seg.module_deps.imports.get("./foo").unwrap();
        assert!(symbols.contains(&TaggedSymbol::new(
            Symbol::named("foo"),
            SymbolTags::default(),
        )));
        assert!(symbols.contains(&TaggedSymbol::new(
            Symbol::named("bar"),
            SymbolTags::default(),
        )));
    }

    #[test]
    fn stmt_with_dynamic_import() {
        let segments = segment("const m = import('./lazy');");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg.module_deps.dynamic_imports.contains_key("./lazy"));
        let locals: Vec<&str> = seg.variables.get_locals().map(|a| a.as_ref()).collect();
        assert!(locals.contains(&"m"));
    }

    #[test]
    fn export_decl_segment() {
        let segments = segment("export const answer = 42;");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg
            .module_deps
            .exports_locals
            .contains_key(&ExportedSymbol::Named("answer".into())));
    }

    #[test]
    fn export_default_segment() {
        let segments = segment("export default function greet() {}");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg
            .module_deps
            .exports_locals
            .contains_key(&ExportedSymbol::Default));
    }

    // ── Side-effect imports, re-exports, dynamic imports ───────────────

    #[test]
    fn side_effect_import() {
        let segments = segment("import './polyfill';");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg.module_deps.executed_paths.contains("./polyfill"));
        assert!(seg.module_deps.imports.is_empty());
    }

    #[test]
    fn re_export_named() {
        let segments = segment("export { foo as bar } from './source';");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        let re_exports = seg.module_deps.exports_from.get("./source").unwrap();
        assert!(re_exports.contains(&ReExportedSymbol {
            imported_as: ImportTarget::ExportedSymbol(ExportedSymbol::Named("foo".into())),
            exported_as: Some(ExportedSymbol::Named("bar".into())),
        }));
    }

    #[test]
    fn re_export_star() {
        let segments = segment("export * from './utils';");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        let re_exports = seg.module_deps.exports_from.get("./utils").unwrap();
        assert!(re_exports.contains(&ReExportedSymbol {
            imported_as: ImportTarget::Namespace,
            exported_as: None,
        }));
    }

    #[test]
    fn require_in_statement() {
        let segments = segment("const lib = require('some-lib');");
        assert_eq!(segments.len(), 1);

        let seg = &segments[0];
        assert!(seg.module_deps.dynamic_imports.is_empty());
        assert!(seg.module_deps.requires.contains("some-lib"));
    }

    #[test]
    fn mixed_file_comprehensive() {
        let segments = segment(
            r#"
            import './setup';
            import { helper } from './helpers';
            const val = helper();
            export { val };
            export * from './re-export-source';
            const lazy = import('./chunk');
            "#,
        );
        // 6 top-level items
        assert_eq!(segments.len(), 6);

        // Segment 0: side-effect import
        assert!(segments[0].module_deps.executed_paths.contains("./setup"));

        // Segment 1: named import
        assert!(segments[1].module_deps.imports.contains_key("./helpers"));

        // Segment 2: const val = helper()
        assert!(segments[2].module_deps.imports.is_empty());
        assert!(segments[2].module_deps.exports_locals.is_empty());
        let locals: Vec<&str> = segments[2]
            .variables
            .get_locals()
            .map(|a| a.as_ref())
            .collect();
        assert!(locals.contains(&"val"));

        // Segment 3: export { val }
        assert!(segments[3]
            .module_deps
            .exports_locals
            .contains_key(&ExportedSymbol::Named("val".into())));

        // Segment 4: export * from ...
        assert!(segments[4]
            .module_deps
            .exports_from
            .contains_key("./re-export-source"));

        // Segment 5: dynamic import
        assert!(segments[5]
            .module_deps
            .dynamic_imports
            .contains_key("./chunk"));
    }
}
