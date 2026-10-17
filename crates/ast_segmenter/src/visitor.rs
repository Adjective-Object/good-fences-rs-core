use crate::variables::{HoistingLevel, VariableScope};
use compact_str::CompactString;
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

/// Map `SymbolFlags` to `HoistingLevel`.
fn hoisting_for_flags(flags: SymbolFlags) -> HoistingLevel {
    if flags.contains(SymbolFlags::Import) {
        HoistingLevel::ImportHoisting
    } else if flags.contains(SymbolFlags::Function) {
        HoistingLevel::FunctionHoisting
    } else {
        HoistingLevel::LetConstHoisting
    }
}

/// Compute `VariableScope` for every top-level `Statement` in one pass.
/// Returned vec has one entry per `program.body[i]` in order.
///
/// For a file with N top-level statements and M total symbols/references,
/// this is O(N + M log N) instead of O(N·M).
fn variables_for_top_level_statements<'a>(
    semantic: &Semantic<'a>,
    body: &[Statement<'a>],
) -> Vec<VariableScope> {
    let stmt_spans: Vec<(u32, u32)> = body
        .iter()
        .map(|s| (s.span().start, s.span().end))
        .collect();
    let mut out: Vec<VariableScope> = (0..body.len()).map(|_| VariableScope::default()).collect();

    let find_stmt = |offset: u32| -> Option<usize> {
        // partition_point on stmt_spans by `end`; check that `start <= offset < end`.
        let i = stmt_spans.partition_point(|(_, end)| *end <= offset);
        stmt_spans
            .get(i)
            .and_then(|(start, end)| (offset >= *start && offset < *end).then_some(i))
    };

    let scoping = semantic.scoping();
    let program_scope = scoping.root_scope_id();
    let bindings = scoping.get_bindings(program_scope);

    // Bindings: walk the program scope (top-level decls) and bucket by statement.
    for (name, &symbol_id) in bindings.iter() {
        let decl_span = scoping.symbol_span(symbol_id);
        let Some(stmt_idx) = find_stmt(decl_span.start) else {
            continue;
        };
        let hoisting = hoisting_for_flags(scoping.symbol_flags(symbol_id));
        out[stmt_idx].insert_local(CompactString::new(name.as_str()), hoisting);
    }

    // Escaped (resolved): for each root binding, check if any of its references
    // fall in a different statement than the binding's declaration.
    for (_name, &symbol_id) in bindings.iter() {
        let decl_stmt = find_stmt(scoping.symbol_span(symbol_id).start);
        let sym_name = scoping.symbol_name(symbol_id);

        for reference in scoping.get_resolved_references(symbol_id) {
            let ref_span = semantic.reference_span(reference);
            let Some(ref_stmt) = find_stmt(ref_span.start) else {
                continue;
            };
            if Some(ref_stmt) != decl_stmt {
                out[ref_stmt].insert_escaped_reference(
                    CompactString::new(sym_name),
                    reference.is_type() && !reference.is_value(),
                );
            }
        }
    }

    // Escaped (unresolved): globals and other names with no binding.
    for (name, ref_ids) in scoping.root_unresolved_references().iter() {
        let name_str = name.as_str();
        for ref_id in ref_ids.iter() {
            let reference = scoping.get_reference(*ref_id);
            let ref_span = semantic.reference_span(reference);
            if let Some(stmt_idx) = find_stmt(ref_span.start) {
                out[stmt_idx].insert_escaped_reference(
                    CompactString::new(name_str),
                    reference.is_type() && !reference.is_value(),
                );
            }
        }
    }

    out
}

fn statement_to_segment<'a>(
    file_logger: &impl SrcFileLogger,
    comments: &[oxc_ast::Comment],
    source: &str,
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
    variables: VariableScope,
) -> Option<Segment> {
    let span = stmt.span();
    match stmt {
        // Module declaration statements — run the exports visitor
        Statement::ImportDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => {
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
            let imports_and_requires =
                import_require_expr::find_imports_and_requires(semantic, stmt);

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
    let mut per_stmt_variables = variables_for_top_level_statements(semantic, &program.body);

    program
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, stmt)| {
            // Take ownership of the pre-computed VariableScope for this statement.
            let variables = std::mem::take(&mut per_stmt_variables[i]);
            statement_to_segment(
                logger,
                &program.comments,
                program.source_text,
                semantic,
                stmt,
                variables,
            )
        })
        .collect()
}

/// Fail rather than returning a partial statement list under bundler policy.
/// Callers must first validate parser and semantic diagnostics, as `parse_module` does.
pub fn try_segment_file<'a>(
    logger: &impl SrcFileLogger,
    program: &Program<'a>,
    semantic: &Semantic<'a>,
) -> eyre::Result<Vec<Segment>> {
    for stmt in &program.body {
        let rejected = match stmt {
            Statement::WithStatement(_) => Some("with"),
            Statement::ReturnStatement(_) => Some("top-level return"),
            Statement::BreakStatement(_) => Some("top-level break"),
            Statement::ContinueStatement(_) => Some("top-level continue"),
            Statement::TSImportEqualsDeclaration(_) => Some("TypeScript import assignment"),
            Statement::TSExportAssignment(_) => Some("TypeScript export assignment"),
            Statement::TSNamespaceExportDeclaration(_) => Some("TypeScript namespace export"),
            _ => None,
        };
        if let Some(kind) = rejected {
            let span = stmt.span();
            eyre::bail!(
                "unsupported {kind} statement at bytes {}..{}",
                span.start,
                span.end
            );
        }
    }
    let segments = segment_file(logger, program, semantic);
    eyre::ensure!(
        segments.len() == program.body.len(),
        "statement extraction rejected part of the program"
    );
    Ok(segments)
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
    fn segment_spans_are_zero_based_end_exclusive_utf8_bytes() {
        let statements = ["export const café = 'é';", "export const next = '🦀';"];
        let source = statements.join("\n");
        let segments = segment(&source);
        assert_eq!(segments.len(), statements.len());

        let mut start = 0;
        for (seg, statement) in segments.iter().zip(statements) {
            let end = start + statement.len();
            assert_eq!(seg.span.start as usize, start);
            assert_eq!(seg.span.end as usize, end);
            assert_eq!(&source[start..end], statement);
            start = end + 1;
        }
        assert_eq!(segments.last().unwrap().span.end as usize, source.len());
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

    // ── Per-statement variable scoping ────────────────────────────────

    #[test]
    fn per_statement_locals_are_bucketed() {
        let segments = segment(
            r#"
            import { foo } from './foo';
            const x = 1;
            function bar() {}
            "#,
        );
        assert_eq!(segments.len(), 3);

        // Segment 0: import → declares `foo` with ImportHoisting
        let locals0: Vec<&str> = segments[0].variables.get_locals().map(|s| s.as_ref()).collect();
        assert!(locals0.contains(&"foo"), "expected 'foo' in segment 0 locals");
        assert_eq!(
            segments[0].variables.get_locals_with_hoisting().find(|(n, _)| n.as_str() == "foo").map(|(_, h)| h),
            Some(HoistingLevel::ImportHoisting)
        );

        // Segment 1: const → declares `x` with LetConstHoisting
        let locals1: Vec<&str> = segments[1].variables.get_locals().map(|s| s.as_ref()).collect();
        assert!(locals1.contains(&"x"), "expected 'x' in segment 1 locals");
        assert!(!locals1.contains(&"foo"), "'foo' should not be in segment 1");

        // Segment 2: function → declares `bar` with FunctionHoisting
        let locals2: Vec<&str> = segments[2].variables.get_locals().map(|s| s.as_ref()).collect();
        assert!(locals2.contains(&"bar"), "expected 'bar' in segment 2 locals");
        assert_eq!(
            segments[2].variables.get_locals_with_hoisting().find(|(n, _)| n.as_str() == "bar").map(|(_, h)| h),
            Some(HoistingLevel::FunctionHoisting)
        );
    }

    #[test]
    fn escaped_symbols_cross_statement() {
        let segments = segment(
            r#"
            import { foo } from './foo';
            const x = foo();
            "#,
        );
        assert_eq!(segments.len(), 2);

        // Segment 0 (import): `foo` is local, not escaped
        let escaped0: Vec<&str> = segments[0].variables.get_escaped_symbols().map(|s| s.as_ref()).collect();
        assert!(!escaped0.contains(&"foo"));

        // Segment 1 (const x = foo()): `foo` is escaped (declared in segment 0)
        let escaped1: Vec<&str> = segments[1].variables.get_escaped_symbols().map(|s| s.as_ref()).collect();
        assert!(escaped1.contains(&"foo"), "expected 'foo' escaped in segment 1");
    }

    #[test]
    fn escaped_symbols_unresolved_globals() {
        let segments = segment("const x = console.log('hello');");
        assert_eq!(segments.len(), 1);

        let escaped: Vec<&str> = segments[0].variables.get_escaped_symbols().map(|s| s.as_ref()).collect();
        assert!(escaped.contains(&"console"), "expected 'console' as escaped global");
    }

    #[test]
    fn escaped_symbols_preserve_type_only_references() {
        let segments = segment(
            r#"
            interface Shape {}
            class Value {}
            export class Consumer<T extends Shape> implements Shape {
                value: Value;
            }
            export type Query = typeof Value;
            export class Runtime extends Value {}
            export function mixed(value: Value) { return new Value(); }
            export function reversed() { return new Value() as Value; }
            export type GlobalType = MissingType;
            export const globalValue = missingValue();
            "#,
        );
        for (idx, expected) in [
            (2, vec![("Shape", true), ("Value", true)]),
            (3, vec![("Value", true)]),
            (4, vec![("Value", false)]),
            (5, vec![("Value", false)]),
            (6, vec![("Value", false)]),
            (7, vec![("MissingType", true)]),
            (8, vec![("missingValue", false)]),
        ] {
            let mut actual: Vec<_> = segments[idx]
                .variables
                .get_escaped_symbols_with_type_only()
                .map(|(name, type_only)| (name.as_str(), type_only))
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "segment {idx}");
        }
    }

    #[test]
    fn self_reference_not_escaped() {
        let segments = segment("function foo() { return foo; }");
        assert_eq!(segments.len(), 1);

        let locals: Vec<&str> = segments[0].variables.get_locals().map(|s| s.as_ref()).collect();
        assert!(locals.contains(&"foo"));

        let escaped: Vec<&str> = segments[0].variables.get_escaped_symbols().map(|s| s.as_ref()).collect();
        assert!(!escaped.contains(&"foo"), "'foo' should not be escaped in its own statement");
    }

    #[test]
    fn export_function_referencing_import() {
        let segments = segment(
            r#"
            import { helper } from './helpers';
            export function doWork() { return helper(); }
            "#,
        );
        assert_eq!(segments.len(), 2);

        // `helper` is local in segment 0, escaped in segment 1
        let locals0: Vec<&str> = segments[0].variables.get_locals().map(|s| s.as_ref()).collect();
        assert!(locals0.contains(&"helper"));

        let escaped1: Vec<&str> = segments[1].variables.get_escaped_symbols().map(|s| s.as_ref()).collect();
        assert!(escaped1.contains(&"helper"), "expected 'helper' escaped in segment 1");
    }
}
