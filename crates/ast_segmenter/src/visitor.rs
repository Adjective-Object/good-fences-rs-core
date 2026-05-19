use logger_srcfile::SrcFileLogger;
use oxc_ast::ast::{Program, Statement};
use oxc_ast_visit::walk;
use oxc_semantic::Semantic;
use oxc_span::GetSpan;

use crate::{
    raw_module_deps::RawModuleDeps,
    segment_info::Segment,
    variables::{variables_for_top_level_statements, VariableScope},
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

fn statement_to_segment<'a>(
    file_logger: &impl SrcFileLogger,
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
    variables: VariableScope,
) -> Option<Segment> {
    let span = stmt.span();
    match stmt {
        // ── Simple module declarations (no body content with dynamic imports) ─
        Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => {
            let mut exports_visitor = ExportsVisitor::new(file_logger, semantic);
            walk::walk_statement(&mut exports_visitor, stmt);
            let module_deps: RawModuleDeps = exports_visitor.into();
            Some(Segment { module_deps, variables, span })
        }

        // ── Export declarations that may contain expression bodies ────────────
        // These go through ExportsVisitor for exports/re-exports AND through
        // find_imports_and_requires to capture dynamic import() calls in the body.
        Statement::ExportNamedDeclaration(_) | Statement::ExportDefaultDeclaration(_) => {
            let mut exports_visitor = ExportsVisitor::new(file_logger, semantic);
            walk::walk_statement(&mut exports_visitor, stmt);
            let mut module_deps: RawModuleDeps = exports_visitor.into();
            // Also find any dynamic imports embedded in the exported value.
            let imports_and_requires =
                import_require_expr::find_imports_and_requires(semantic, stmt);
            module_deps
                .dynamic_imports
                .extend(imports_and_requires.imported_paths.names());
            module_deps
                .requires
                .extend(imports_and_requires.require_paths.names().into_keys());
            Some(Segment { module_deps, variables, span })
        }

        // ── Regular statements and declarations ──────────────────────────────
        Statement::BlockStatement(_)
        | Statement::DebuggerStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::EmptyStatement(_)
        | Statement::ExpressionStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::ForStatement(_)
        | Statement::IfStatement(_)
        | Statement::LabeledStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::TryStatement(_)
        | Statement::WhileStatement(_)
        | Statement::VariableDeclaration(_)
        | Statement::FunctionDeclaration(_)
        | Statement::ClassDeclaration(_)
        | Statement::TSTypeAliasDeclaration(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::TSEnumDeclaration(_)
        | Statement::TSModuleDeclaration(_)
        | Statement::TSGlobalDeclaration(_) => {
            let imports_and_requires = import_require_expr::find_imports_and_requires(semantic, stmt);

            let dynamic_imports = imports_and_requires.imported_paths.names().into_iter().collect();
            let requires = imports_and_requires.require_paths.names().into_keys().collect();

            let module_deps = RawModuleDeps {
                dynamic_imports,
                requires,
                ..Default::default()
            };

            Some(Segment { module_deps, variables, span })
        }

        // ── Error cases ──────────────────────────────────────────────────────
        Statement::WithStatement(_) => {
            file_logger.src_error(span, StatementToSegmentError::WithStatmentUnsupported);
            None
        }
        Statement::ReturnStatement(_) => {
            file_logger.src_error(
                span,
                StatementToSegmentError::StatementUnexpectedInModuleScope(RETURN),
            );
            None
        }
        Statement::BreakStatement(_) => {
            file_logger.src_error(
                span,
                StatementToSegmentError::StatementUnexpectedInModuleScope(BREAK),
            );
            None
        }
        Statement::ContinueStatement(_) => {
            file_logger.src_error(
                span,
                StatementToSegmentError::StatementUnexpectedInModuleScope(CONTINUE),
            );
            None
        }

        // ── TSExportAssignment / TSNamespaceExportDeclaration ────────────────
        // These are uncommon TS-specific module forms; treat as regular stmts.
        Statement::TSExportAssignment(_) | Statement::TSNamespaceExportDeclaration(_) => {
            Some(Segment {
                module_deps: Default::default(),
                variables,
                span,
            })
        }
    }
}

/// Segment a parsed module into a list of `Segment`s — one per top-level statement.
/// Each segment combines variable scope analysis with module dependency extraction.
pub fn segment_file<'a>(
    logger: &impl SrcFileLogger,
    program: &Program<'a>,
    semantic: &Semantic<'a>,
) -> Vec<Segment> {
    let variables = variables_for_top_level_statements(semantic, &program.body);
    program
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, stmt)| {
            statement_to_segment(logger, semantic, stmt, variables[i].clone())
        })
        .collect()
}


#[cfg(test)]
mod test {
    use super::*;
    use crate::raw_module_deps::{Symbol, SymbolTags, TaggedSymbol};
    use crate::{ExportedSymbol, ImportTarget, ReExportedSymbol};

    /// Parse TypeScript source and return the segments produced by `segment_file`.
    fn segment(src: &str) -> Vec<Segment> {
        let allocator = oxc_allocator::Allocator::default();
        let ret = oxc_utils_parse::parse_ts(&allocator, src);
        let semantic_ret = oxc_semantic::SemanticBuilder::new().build(&ret.program);

        let logger = logger::StdioLogger::new();
        let file_logger = logger_srcfile::WrapFileLogger::new("test.ts", src.to_string(), &logger);
        segment_file(&file_logger, &ret.program, &semantic_ret.semantic)
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
            tags: SymbolTags::default(),
            span: Default::default(),
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
            tags: SymbolTags::default(),
            span: Default::default(),
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
