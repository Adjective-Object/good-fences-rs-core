use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, Expression,
    FormalParameters, ImportExpression, PropertyKey, Statement,
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

/// Checks if `ident` is a reference to the global `require` (i.e., unresolved).
fn is_global_require<'a>(semantic: &Semantic<'a>, ident: &oxc_ast::ast::IdentifierReference<'a>) -> bool {
    if ident.name != "require" {
        return false;
    }
    let Some(ref_id) = ident.reference_id.get() else {
        return false;
    };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}

/// Extract the static string value from a string-literal import argument list.
fn args_as_string_literal<'a>(args: &[Argument<'a>]) -> Option<String> {
    match args.first()? {
        Argument::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// Extract named import symbols from the first parameter of a `.then()` callback.
///
/// Matches: `({ a, b, c }) => ...` and `function({ a, b, c }) { ... }`.
/// Returns the static key names (what the module exports), not the local binding names.
fn extract_then_arg_names<'a>(args: &[Argument<'a>]) -> Option<Vec<Symbol>> {
    let first_arg = args.first()?;

    let params: &FormalParameters = match first_arg {
        Argument::ArrowFunctionExpression(arrow) => &arrow.params,
        Argument::FunctionExpression(f) => &f.params,
        _ => return None,
    };

    let first_param = params.items.first()?;
    let BindingPattern::ObjectPattern(obj) = &first_param.pattern else {
        return None;
    };

    let names: Vec<Symbol> = obj
        .properties
        .iter()
        .filter_map(|prop| {
            match &prop.key {
                PropertyKey::StaticIdentifier(id) => {
                    Some(Symbol::Named(Name::from(id.name.as_str())))
                }
                _ => None,
            }
        })
        .collect();

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

struct ImportRequireVisitor<'s, 'a> {
    semantic: &'s Semantic<'a>,
    result: ImportsAndRequires,
}

impl<'s, 'a> ImportRequireVisitor<'s, 'a> {
    fn new(semantic: &'s Semantic<'a>) -> Self {
        Self {
            semantic,
            result: Default::default(),
        }
    }
}

impl<'s, 'a> Visit<'a> for ImportRequireVisitor<'s, 'a> {
    /// Handle `import('foo')` — adds Namespace to imported_paths.
    ///
    /// Intentionally does NOT walk children; the source of import() can only be
    /// a static string in the cases we track.
    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        if let Expression::StringLiteral(path) = &import.source {
            self.result
                .imported_paths
                .entry(path.value.to_string())
                .or_default()
                .insert(Symbol::Namespace);
        }
    }

    /// Handle `require('foo')` and `import('foo').then(({a,b}) => ...)`.
    ///
    /// Children are walked first so that nested `import()` expressions have
    /// already been recorded before we check for the `.then()` upgrade pattern.
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // Visit children first (processes any nested import() expressions)
        walk::walk_call_expression(self, call);

        // require('foo') — only if it's an unresolved global `require`
        if let Expression::Identifier(ident) = &call.callee {
            if is_global_require(self.semantic, ident) {
                if let Some(path) = args_as_string_literal(&call.arguments) {
                    self.result.require_paths.insert(path, Symbol::Default);
                }
            }
        }

        // import('foo').then(({a,b,c}) => { ... })
        //   — detect the pattern and replace the Namespace entry with named imports
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if member.property.name == "then" {
                if let Expression::ImportExpression(import_expr) = &member.object {
                    if let Expression::StringLiteral(path) = &import_expr.source {
                        let path_str = path.value.to_string();
                        if let Some(names) = extract_then_arg_names(&call.arguments) {
                            let entry = self.result.imported_paths.entry(path_str).or_default();
                            entry.remove(&Symbol::Namespace);
                            for name in names {
                                entry.insert(name);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Walk a single statement and collect all dynamic `import()` and `require()` calls.
pub fn find_imports_and_requires<'a>(
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
) -> ImportsAndRequires {
    let mut visitor = ImportRequireVisitor::new(semantic);
    walk::walk_statement(&mut visitor, stmt);
    visitor.result
}

#[cfg(test)]
mod test {
    use crate::raw_module_deps::Symbol;

    use super::find_imports_and_requires;
    use ahashmap::{AHashMap, AHashSet};
    use oxc_allocator::Allocator;
    use oxc_semantic::SemanticBuilder;
    use oxc_utils_parse::parse_ts;

    fn test_discovers_import_expr(
        source: &str,
        expected_imported_paths: AHashMap<String, AHashSet<Symbol>>,
        expected_require_paths: AHashMap<String, AHashSet<Symbol>>,
    ) {
        let allocator = Allocator::default();
        let ret = parse_ts(&allocator, source);
        let semantic = SemanticBuilder::new().build(&ret.program).semantic;

        // Aggregate results from all statements
        let mut all_imported: AHashMap<String, AHashSet<Symbol>> = AHashMap::default();
        let mut all_required: AHashMap<String, AHashSet<Symbol>> = AHashMap::default();
        for stmt in &ret.program.body {
            let result = find_imports_and_requires(&semantic, stmt);
            let (imported_paths, require_paths) =
                (result.imported_paths, result.require_paths);
            for (k, v) in imported_paths.names() {
                all_imported.entry(k).or_default().extend(v);
            }
            for (k, v) in require_paths.names() {
                all_required.entry(k).or_default().extend(v);
            }
        }

        assert_eq!(all_imported, expected_imported_paths, "imported_paths mismatch for: {source}");
        assert_eq!(all_required, expected_require_paths, "require_paths mismatch for: {source}");
    }

    fn imap(pairs: &[(&str, Vec<Symbol>)]) -> AHashMap<String, AHashSet<Symbol>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().cloned().collect()))
            .collect()
    }

    #[test]
    fn test_basic_import() {
        test_discovers_import_expr(
            "import('foo')",
            imap(&[("foo", vec![Symbol::Namespace])]),
            Default::default(),
        );
    }

    #[test]
    fn test_basic_require() {
        test_discovers_import_expr(
            "require('foo')",
            Default::default(),
            imap(&[("foo", vec![Symbol::Default])]),
        );
    }

    #[test]
    fn test_import_expr_deep() {
        test_discovers_import_expr(
            "if (true) { import('foo') } else { require('bar') }",
            imap(&[("foo", vec![Symbol::Namespace])]),
            imap(&[("bar", vec![Symbol::Default])]),
        );
    }

    #[test]
    fn test_import_expr_extracts_names_arrow() {
        test_discovers_import_expr(
            "import('foo').then(({a,b,c}) => { console.log(a,b,c) })",
            imap(&[("foo", vec![Symbol::named("a"), Symbol::named("b"), Symbol::named("c")])]),
            Default::default(),
        );
    }

    #[test]
    fn test_import_expr_extracts_names_noarrow() {
        test_discovers_import_expr(
            "import('foo').then(function myfunc({a,b,c}) { console.log(a,b,c) })",
            imap(&[("foo", vec![Symbol::named("a"), Symbol::named("b"), Symbol::named("c")])]),
            Default::default(),
        );
    }
}