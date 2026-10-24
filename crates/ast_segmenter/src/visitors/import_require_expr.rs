use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, Expression, IdentifierReference, Statement,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Semantic;

use crate::{
    name_set::NameSet,
    raw_module_deps::{Name, Symbol},
};

#[derive(Default)]
pub struct ImportsAndRequires {
    pub imported_paths: NameSet<String, Symbol>,
    pub require_paths: NameSet<String, Symbol>,
}

impl ImportsAndRequires {
    /// Checks if the current call expr is a require() or import().then() pattern
    /// and if it is, updates this data structure with the import path and the names that are
    /// being imported.
    pub fn scan_call_expr<'a>(&mut self, semantic: &Semantic<'a>, expr: &CallExpression<'a>) {
        match &expr.callee {
            // require('foo')
            Expression::Identifier(ident) => {
                if is_global_require(semantic, ident) {
                    if let Some(import_path) = args_as_import(&expr.arguments) {
                        self.require_paths.insert(import_path, Symbol::Default);
                    }
                }
            }
            // import('foo').then(({name1, name2}) => {...})
            Expression::StaticMemberExpression(member) => {
                if member.property.name.as_str() != "then" {
                    return;
                }
                // Check if obj is import('foo')
                if let Expression::ImportExpression(import_expr) = &member.object {
                    let imported_path = match extract_string_from_expression(&import_expr.source) {
                        Some(path) => path,
                        None => return,
                    };

                    // args in .then((<args>) => {..}) or .then(function (<args>) {..})
                    let then_arg_obj_pattern = match expr.arguments.first() {
                        Some(arg) => {
                            let arg_expr = match arg {
                                Argument::SpreadElement(_) => return,
                                _ => arg.as_expression().unwrap(),
                            };
                            match extract_generic_function_def_first_arg(arg_expr) {
                                Some(BindingPattern::ObjectPattern(obj_pat)) => obj_pat,
                                _ => return,
                            }
                        }
                        None => return,
                    };

                    // extract names from the object binding pattern
                    let obj_names = then_arg_obj_pattern.properties.iter().filter_map(
                        |prop| -> Option<Symbol> {
                            match prop.key {
                                oxc_ast::ast::PropertyKey::StaticIdentifier(ref ident) => {
                                    Some(Symbol::Named(Name::from(ident.name.as_str())))
                                }
                                _ => None,
                            }
                        },
                    );

                    // store the names (removing any Namespace entry that was added
                    // by visit_import_expression processing the inner import() call)
                    let entry = self.imported_paths.entry(imported_path).or_default();
                    entry.remove(&Symbol::Namespace);
                    for name in obj_names {
                        entry.insert(name);
                    }
                }
            }
            _ => {}
        }
    }

    /// Handle a standalone `import('foo')` expression.
    pub fn scan_import_expr<'a>(&mut self, import_expr: &oxc_ast::ast::ImportExpression<'a>) {
        if let Some(import_path) = extract_string_from_expression(&import_expr.source) {
            self.imported_paths.insert(import_path, Symbol::Namespace);
        }
    }
}

fn is_global_require<'a>(semantic: &Semantic<'a>, ident: &IdentifierReference<'a>) -> bool {
    if ident.name != "require" {
        return false;
    }
    let Some(ref_id) = ident.reference_id.get() else {
        return false;
    };
    semantic
        .scoping()
        .get_reference(ref_id)
        .symbol_id()
        .is_none()
}

fn extract_generic_function_def_first_arg<'a>(
    expr: &'a Expression<'a>,
) -> Option<&'a BindingPattern<'a>> {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => {
            arrow.params.items.first().map(|p| &p.pattern)
        }
        Expression::FunctionExpression(func) => func.params.items.first().map(|p| &p.pattern),
        _ => None,
    }
}

fn extract_string_from_expression(expr: &Expression<'_>) -> Option<String> {
    if let Expression::StringLiteral(s) = expr {
        s.value.as_str().map(str::to_owned)
    } else {
        None
    }
}

fn args_as_import(args: &oxc_allocator::Vec<'_, Argument<'_>>) -> Option<String> {
    let first = args.first()?;
    match first {
        Argument::StringLiteral(s) => s.value.as_str().map(str::to_owned),
        _ => None,
    }
}

/// Visitor that wraps `ImportsAndRequires` with a reference to `Semantic`,
/// implementing oxc's `Visit<'a>` trait.
struct ImportsAndRequiresVisitor<'v, 'a> {
    inner: ImportsAndRequires,
    semantic: &'v Semantic<'a>,
}

impl<'v, 'a> Visit<'a> for ImportsAndRequiresVisitor<'v, 'a> {
    fn visit_call_expression(&mut self, call_expr: &CallExpression<'a>) {
        walk::walk_call_expression(self, call_expr);
        self.inner.scan_call_expr(self.semantic, call_expr);
    }

    fn visit_import_expression(&mut self, import_expr: &oxc_ast::ast::ImportExpression<'a>) {
        walk::walk_import_expression(self, import_expr);
        self.inner.scan_import_expr(import_expr);
    }
}

pub fn find_imports_and_requires<'a>(
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
) -> ImportsAndRequires {
    let mut visitor = ImportsAndRequiresVisitor {
        inner: ImportsAndRequires::default(),
        semantic,
    };
    walk::walk_statement(&mut visitor, stmt);
    visitor.inner
}

pub(crate) fn find_declarator_imports_and_requires<'a>(
    semantic: &Semantic<'a>,
    declarator: &oxc_ast::ast::VariableDeclarator<'a>,
) -> ImportsAndRequires {
    let mut visitor = ImportsAndRequiresVisitor {
        inner: ImportsAndRequires::default(),
        semantic,
    };
    visitor.visit_variable_declarator(declarator);
    visitor.inner
}

#[cfg(test)]
mod test {
    use crate::raw_module_deps::Symbol;

    use super::*;
    use ahashmap::AHashMap;

    use test_tmpdir::amap2;

    fn test_discovers_import_expr(
        source: &str,
        expected_imported_paths: AHashMap<&str, Vec<Symbol>>,
        expected_require_paths: AHashMap<&str, Vec<Symbol>>,
    ) {
        let allocator = oxc_allocator::Allocator::default();
        let ret = oxc_utils_parse::parse_ts(&allocator, source);
        assert!(
            ret.diagnostics.is_empty(),
            "parse errors: {:?}",
            ret.diagnostics
        );
        let semantic_ret = oxc_semantic::SemanticBuilder::new().build(&ret.program);
        let semantic = &semantic_ret.semantic;

        let mut result = ImportsAndRequires::default();
        for stmt in &ret.program.body {
            let stmt_result = find_imports_and_requires(semantic, stmt);
            for (k, v) in stmt_result.imported_paths.names() {
                let entry = result.imported_paths.entry(k).or_default();
                for sym in v {
                    entry.insert(sym);
                }
            }
            for (k, v) in stmt_result.require_paths.names() {
                let entry = result.require_paths.entry(k).or_default();
                for sym in v {
                    entry.insert(sym);
                }
            }
        }

        assert_eq!(
            result.imported_paths.names(),
            expected_imported_paths
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().cloned().collect()))
                .collect(),
        );

        assert_eq!(
            result.require_paths.names(),
            expected_require_paths
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().cloned().collect()))
                .collect(),
        );
    }

    #[test]
    fn test_basic_import() {
        test_discovers_import_expr(
            "import('foo')",
            amap2![
                "foo" => vec![Symbol::Namespace]
            ],
            Default::default(),
        );
    }

    #[test]
    fn test_basic_require() {
        test_discovers_import_expr(
            "require('foo')",
            Default::default(),
            amap2![
                "foo" => vec![Symbol::Default]
            ],
        );
    }

    #[test]
    fn test_import_expr_deep() {
        test_discovers_import_expr(
            "if (true) { import('foo') } else { require('bar') }",
            amap2![
                "foo" => vec![Symbol::Namespace]
            ],
            amap2![
                "bar" => vec![Symbol::Default]
            ],
        );
    }

    #[test]
    fn test_import_expr_extracts_names_arrow() {
        test_discovers_import_expr(
            "import('foo').then(({a,b,c}) => { console.log(a,b,c) })",
            amap2![
                "foo" => vec![
                    Symbol::named("a"),
                    Symbol::named("b"),
                    Symbol::named("c")]
            ],
            Default::default(),
        );
    }

    #[test]
    fn test_import_expr_extracts_names_noarrow() {
        test_discovers_import_expr(
            "import('foo').then(function myfunc({a,b,c}) { console.log(a,b,c) })",
            amap2![
                "foo" => vec![
                    Symbol::named("a"),
                    Symbol::named("b"),
                    Symbol::named("c")]
            ],
            Default::default(),
        );
    }
}
