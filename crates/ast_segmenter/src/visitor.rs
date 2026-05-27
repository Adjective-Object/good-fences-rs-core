use crate::variables::{HoistingLevel, VariableScope};
use logger_srcfile::SrcFileLogger;
use oxc_ast::ast::{Program, Statement};
use oxc_ast_visit::walk;
use oxc_semantic::{Semantic, SymbolFlags};
use oxc_span::GetSpan;

use crate::{
    raw_module_deps::RawModuleDeps,
    segment_info::Segment,
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

/// Temporary shim: extract a `VariableScope` from oxc `Semantic` for the
/// root scope bindings. Replaces `ast_name_tracker::visitor::find_names`.
/// Full deletion of `ast_name_tracker` happens in phase 5.
fn find_names_from_semantic(semantic: &Semantic<'_>) -> VariableScope {
    let scoping = semantic.scoping();
    let root_scope = scoping.root_scope_id();
    let mut scope = VariableScope::new();
    let bindings = scoping.get_bindings(root_scope);
    for (name, &symbol_id) in bindings.iter() {
        let flags = scoping.symbol_flags(symbol_id);
        let hoisting = if flags.contains(SymbolFlags::Import) {
            HoistingLevel::ImportHoisting
        } else if flags.contains(SymbolFlags::Function) {
            HoistingLevel::FunctionHoisting
        } else {
            HoistingLevel::LetConstHoisting
        };
        scope.insert_local_str(name.as_str(), hoisting);
    }
    scope
}

fn statement_to_segment<'a>(
    file_logger: &impl SrcFileLogger,
    comments: &[oxc_ast::Comment],
    source: &str,
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
) -> Option<Segment> {
    let span = stmt.span();
    match stmt {
        // Module declaration statements — run the exports visitor
        Statement::ImportDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => {
            let variables = find_names_from_semantic(semantic);

            let mut exports_visitor = ExportsVisitor::new(file_logger, comments, source, semantic);
            walk::walk_statement(&mut exports_visitor, stmt);
            let module_deps: RawModuleDeps = exports_visitor.into();

            Some(Segment {
                module_deps,
                variables,
                span,
            })
        }
        // Error statements
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
        // Regular statements — extract dynamic imports/requires
        _ => {
            let variables = find_names_from_semantic(semantic);
            let imports_and_requires = import_require_expr::find_imports_and_requires(semantic, stmt);

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

            Some(Segment {
                module_deps,
                variables,
                span,
            })
        }
    }
}

/// Segment a parsed program into a list of `Segment`s — one per top-level
/// `Statement`. Each segment combines variable scope analysis with module
/// dependency extraction.
pub fn segment_file<'a>(
    logger: &impl SrcFileLogger,
    program: &Program<'a>,
    semantic: &Semantic<'a>,
) -> Vec<Segment> {
    program
        .body
        .iter()
        .filter_map(|stmt| {
            statement_to_segment(
                logger,
                &program.comments,
                program.source_text,
                semantic,
                stmt,
            )
        })
        .collect()
}


#[cfg(test)]
mod test {
    use super::*;
    use crate::raw_module_deps::{Symbol, SymbolTags, TaggedSymbol};
    use crate::{ExportedSymbol, ImportTarget, ReExportedSymbol};

    /// Parse source and return the segments produced by `segment_file`.
    fn segment(src: &str) -> Vec<Segment> {
        let allocator = oxc_allocator::Allocator::default();
        let ret = oxc_utils_parse::parse_file(&allocator, src, std::path::Path::new("test.ts"));
        assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);
        let semantic_ret = oxc_semantic::SemanticBuilder::new().build(&ret.program);
        let semantic = &semantic_ret.semantic;
        let logger = logger::StdioLogger::new();
        let file_logger = logger_srcfile::WrapFileLogger::new("test.ts", src.to_owned(), &logger);
        segment_file(&file_logger, &ret.program, semantic)
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
