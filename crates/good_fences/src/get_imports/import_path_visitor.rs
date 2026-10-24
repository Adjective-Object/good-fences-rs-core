use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, CallExpression, ExportFromDeclaration, Expression, IdentifierReference,
    ImportDeclaration, ImportDeclarationSpecifier, ImportExpression, ModuleExportName,
    TSImportEqualsDeclaration, TSModuleReference,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Semantic;

pub struct ImportPathVisitor<'s, 'a> {
    pub require_paths: HashSet<String>,
    pub import_paths: HashSet<String>,
    pub imports_map: HashMap<String, HashSet<String>>,
    semantic: &'s Semantic<'a>,
}

impl std::fmt::Debug for ImportPathVisitor<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportPathVisitor")
            .field("require_paths", &self.require_paths)
            .field("import_paths", &self.import_paths)
            .field("imports_map", &self.imports_map)
            .finish_non_exhaustive()
    }
}

impl<'s, 'a> ImportPathVisitor<'s, 'a> {
    pub fn new(semantic: &'s Semantic<'a>) -> Self {
        Self {
            require_paths: HashSet::new(),
            import_paths: HashSet::new(),
            imports_map: HashMap::new(),
            semantic,
        }
    }
}

impl<'s, 'a> Visit<'a> for ImportPathVisitor<'s, 'a> {
    fn visit_export_from_declaration(&mut self, export: &ExportFromDeclaration<'a>) {
        walk::walk_export_from_declaration(self, export);

        let Some(source_str) = export.source.value.as_str().map(str::to_owned) else {
            return;
        };
        let mut specifiers: HashSet<String> = export
            .specifiers
            .iter()
            .filter_map(|spec| -> Option<String> {
                let name = module_export_name_to_str(&spec.local);
                Some(name.to_string())
            })
            .collect();

        if let Some(imports) = self.imports_map.get_mut(&source_str) {
            specifiers.drain().for_each(|x| {
                imports.insert(x);
            });
        } else {
            self.imports_map.insert(source_str, specifiers);
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        walk::walk_ts_import_equals_declaration(self, decl);
        if let TSModuleReference::ExternalModuleReference(module_ref) = &decl.module_reference {
            if let Some(source) = module_ref.expression.value.as_str() {
                self.import_paths.insert(source.to_owned());
            }
        }
    }

    fn visit_call_expression(&mut self, expr: &CallExpression<'a>) {
        walk::walk_call_expression(self, expr);

        // require('foo') — only if `require` is the global (not shadowed)
        if let Expression::Identifier(ident) = &expr.callee {
            if is_global_require(self.semantic, ident) {
                if let Some(import_path) = extract_argument_value(&expr.arguments) {
                    self.require_paths.insert(import_path);
                }
            }
        }
    }

    fn visit_import_expression(&mut self, expr: &ImportExpression<'a>) {
        walk::walk_import_expression(self, expr);

        // import('foo')
        if let Expression::StringLiteral(s) = &expr.source {
            if let Some(source) = s.value.as_str() {
                self.import_paths.insert(source.to_owned());
            }
        }
    }

    fn visit_import_declaration(&mut self, node: &ImportDeclaration<'a>) {
        walk::walk_import_declaration(self, node);
        let Some(source_path) = node.source.value.as_str().map(str::to_owned) else {
            return;
        };
        if let Some(specifiers) = &node.specifiers {
            if let Some(imported_names) = self.imports_map.get_mut(&source_path) {
                for spec in specifiers {
                    append_imported_names(spec, imported_names);
                }
            } else {
                let mut imported_names = HashSet::new();
                for spec in specifiers {
                    append_imported_names(spec, &mut imported_names);
                }
                self.imports_map.insert(source_path, imported_names);
            }
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

fn module_export_name_to_str<'a>(name: &ModuleExportName<'a>) -> &'a str {
    name.name().as_str()
}

fn append_imported_names(
    spec: &ImportDeclarationSpecifier<'_>,
    imported_names: &mut HashSet<String>,
) {
    match spec {
        ImportDeclarationSpecifier::ImportSpecifier(named) => match &named.imported {
            ModuleExportName::IdentifierName(ident) => {
                imported_names.insert(ident.name.to_string());
            }
            ModuleExportName::IdentifierReference(ident) => {
                imported_names.insert(ident.name.to_string());
            }
            ModuleExportName::StringLiteral(s) => {
                if let Some(name) = s.value.as_str() {
                    imported_names.insert(name.to_owned());
                }
            }
        },
        ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
            imported_names.insert("default".to_string());
        }
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
            // namespace import (import * as ns) — no specific name to record
        }
    }
}

fn extract_argument_value(args: &oxc_allocator::Vec<'_, Argument<'_>>) -> Option<String> {
    let first = args.first()?;
    match first {
        Argument::StringLiteral(s) => s.value.as_str().map(str::to_owned),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use oxc_allocator::Allocator;
    use oxc_ast_visit::Visit;

    use super::ImportPathVisitor;

    macro_rules! with_parsed_visitor {
        ($source:expr, |$visitor:ident| $body:expr) => {{
            let allocator = Allocator::default();
            let ret = oxc_utils_parse::parse_ts(&allocator, $source);
            assert!(
                ret.diagnostics.is_empty(),
                "parse errors: {:?}",
                ret.diagnostics
            );
            let semantic_ret = oxc_semantic::SemanticBuilder::new().build(&ret.program);
            let semantic = &semantic_ret.semantic;
            let mut $visitor = ImportPathVisitor::new(semantic);
            $visitor.visit_program(&ret.program);
            $body
        }};
    }

    #[test]
    fn text_export_from() {
        with_parsed_visitor!(
            r#"export { default as a, foo as bar } from './foo'"#,
            |visitor| {
                let expected_map: HashMap<String, HashSet<String>> = HashMap::from([(
                    "./foo".to_owned(),
                    HashSet::from(["default".to_owned(), "foo".to_owned()]),
                )]);
                assert_eq!(expected_map, visitor.imports_map);
            }
        );
    }

    #[test]
    fn test_require_imports() {
        with_parsed_visitor!(r#"require('hello-world')"#, |visitor| {
            let expected_require_set = HashSet::from(["hello-world".to_string()]);
            assert_eq!(expected_require_set, visitor.require_paths);
        });
    }

    #[test]
    fn test_import_call() {
        with_parsed_visitor!(
            r#"
                import('foo')
            "#,
            |visitor| {
                let expected_import_paths = HashSet::from(["foo".to_string()]);
                assert_eq!(expected_import_paths, visitor.import_paths);
            }
        );
    }

    #[test]
    fn test_nested_import_call() {
        with_parsed_visitor!(
            r#"
                import(import('import_subrequire').default + '/parent')
            "#,
            |visitor| {
                let expected_import_paths = HashSet::from(["import_subrequire".to_string()]);
                assert_eq!(expected_import_paths, visitor.import_paths);
            }
        );
    }

    #[test]
    fn test_require_shadowing() {
        with_parsed_visitor!(
            r#"
                require("foo");
                (function() {
                  const require = console.log;
                  require("bar");
                })();
                require("original")
            "#,
            |visitor| {
                let expected_require_set =
                    HashSet::from(["foo".to_string(), "original".to_string()]);
                assert_eq!(expected_require_set, visitor.require_paths);
            }
        );
    }

    #[test]
    fn test_imports() {
        with_parsed_visitor!(
            r#"
            import foo from './bar';
            "#,
            |visitor| {
                let expected_import_map =
                    HashMap::from([("./bar".to_string(), HashSet::from(["default".to_string()]))]);
                assert_eq!(expected_import_map, visitor.imports_map);
            }
        );
    }

    #[test]
    fn trest_import_with_satisfies() {
        with_parsed_visitor!(
            r#"
            import foo from './bar';
            foo satisfies never;
            "#,
            |visitor| {
                let expected_import_map =
                    HashMap::from([("./bar".to_string(), HashSet::from(["default".to_string()]))]);
                assert_eq!(expected_import_map, visitor.imports_map);
            }
        );
    }

    #[test]
    fn test_imports_specifiers() {
        with_parsed_visitor!(
            r#"
            import {foo, bar} from './bar';
            "#,
            |visitor| {
                let expected_import_map = HashMap::from([(
                    "./bar".to_string(),
                    HashSet::from(["foo".to_string(), "bar".to_string()]),
                )]);
                assert_eq!(expected_import_map, visitor.imports_map);
            }
        );
    }

    #[test]
    fn test_require_redefinition() {
        // `var require` hoists to the top of the function scope, so oxc_semantic
        // correctly resolves _all_ `require` references (including those lexically
        // before the declaration) to the local variable, not the global.
        with_parsed_visitor!(
            r#"
                require('before_definition')
                var require = function(){}
                require('after_definition')
            "#,
            |visitor| {
                let expected_require_set: HashSet<String> = HashSet::new();
                assert_eq!(expected_require_set, visitor.require_paths);
            }
        );
    }

    #[test]
    fn test_require_inside_call_expr() {
        with_parsed_visitor!(
            r#"
                (function otherFunction() {})(require('arg_subrequire'))
                (require('callee_subrequire'))("foo")
            "#,
            |visitor| {
                let expected_require_set = HashSet::from([
                    "arg_subrequire".to_string(),
                    "callee_subrequire".to_string(),
                ]);
                assert_eq!(expected_require_set, visitor.require_paths);
            }
        );
    }

    #[test]
    fn test_require_inside_require() {
        with_parsed_visitor!(
            r#"
                require(require('require_subrequire').default + '/parent')
            "#,
            |visitor| {
                let expected_require_set = HashSet::from(["require_subrequire".to_string()]);
                assert_eq!(expected_require_set, visitor.require_paths);
            }
        );
    }
}
