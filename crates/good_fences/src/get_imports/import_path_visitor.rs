use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, CallExpression, Expression, ExportNamedDeclaration, ImportDeclaration,
    ImportDeclarationSpecifier, ImportExpression, ModuleExportName, TSImportEqualsDeclaration,
    TSModuleReference,
};
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Semantic;

pub struct ImportPathVisitor<'s, 'a> {
    pub require_paths: HashSet<String>,
    pub import_paths: HashSet<String>,
    pub imports_map: HashMap<String, HashSet<String>>,
    semantic: &'s Semantic<'a>,
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

/// Returns true if `ident` is an unresolved reference to the global `require`.
fn is_global_require<'a>(
    semantic: &Semantic<'a>,
    ident: &oxc_ast::ast::IdentifierReference<'a>,
) -> bool {
    if ident.name != "require" {
        return false;
    }
    let Some(ref_id) = ident.reference_id.get() else {
        return false;
    };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}

/// Extract the string value from all three `ModuleExportName` variants.
fn module_export_name_str(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::IdentifierName(ident) => ident.name.to_string(),
        ModuleExportName::IdentifierReference(ident) => ident.name.to_string(),
        ModuleExportName::StringLiteral(s) => s.value.to_string(),
    }
}

impl<'s, 'a> Visit<'a> for ImportPathVisitor<'s, 'a> {
    fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
        // Walk children so that any require()/import() inside exported declarations are found.
        walk::walk_export_named_declaration(self, export);

        if let Some(source) = &export.source {
            let source_str = source.value.to_string();
            let specifiers: HashSet<String> = export
                .specifiers
                .iter()
                .map(|spec| module_export_name_str(&spec.local))
                .collect();

            if let Some(imports) = self.imports_map.get_mut(&source_str) {
                for s in specifiers {
                    imports.insert(s);
                }
            } else {
                self.imports_map.insert(source_str, specifiers);
            }
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(emr) = &decl.module_reference {
            self.import_paths.insert(emr.expression.value.to_string());
        }
    }

    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        // Walk children first to handle nested import() expressions.
        walk::walk_import_expression(self, import);
        if let Expression::StringLiteral(path) = &import.source {
            self.import_paths.insert(path.value.to_string());
        }
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // Walk children first so nested require()/import() calls are captured.
        walk::walk_call_expression(self, call);

        if let Expression::Identifier(ident) = &call.callee {
            if is_global_require(self.semantic, ident) {
                if let Some(path) = extract_argument_value(&call.arguments) {
                    self.require_paths.insert(path);
                }
            }
        }
    }

    fn visit_import_declaration(&mut self, node: &ImportDeclaration<'a>) {
        let source_path = node.source.value.to_string();
        let specifiers = node.specifiers.as_deref().map_or(&[][..], |v| v.as_slice());
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

fn append_imported_names(spec: &ImportDeclarationSpecifier, imported_names: &mut HashSet<String>) {
    match spec {
        ImportDeclarationSpecifier::ImportSpecifier(named) => {
            imported_names.insert(module_export_name_str(&named.imported));
        }
        ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
            imported_names.insert("default".to_string());
        }
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
            // Namespace imports (`import * as foo`) carry no specific named specifier.
        }
    }
}

fn extract_argument_value(args: &[Argument]) -> Option<String> {
    match args.first()? {
        Argument::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use oxc_allocator::Allocator;
    use oxc_ast_visit::Visit;
    use oxc_semantic::SemanticBuilder;
    use oxc_utils_parse::parse_ts;

    use super::ImportPathVisitor;

    struct VisitorResult {
        require_paths: HashSet<String>,
        import_paths: HashSet<String>,
        imports_map: HashMap<String, HashSet<String>>,
    }

    fn run(source: &str) -> VisitorResult {
        let allocator = Allocator::default();
        let ret = parse_ts(&allocator, source);
        let semantic = SemanticBuilder::new().build(&ret.program).semantic;
        let mut visitor = ImportPathVisitor::new(&semantic);
        visitor.visit_program(&ret.program);
        VisitorResult {
            require_paths: visitor.require_paths,
            import_paths: visitor.import_paths,
            imports_map: visitor.imports_map,
        }
    }

    #[test]
    fn text_export_from() {
        let result = run(r#"export { default as a, foo as bar } from './foo'"#);
        let expected_map: HashMap<String, HashSet<String>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from(["default".to_owned(), "foo".to_owned()]),
        )]);
        assert_eq!(expected_map, result.imports_map);
    }

    #[test]
    fn test_require_imports() {
        let result = run(r#"require('hello-world')"#);
        let expected_require_set = HashSet::from(["hello-world".to_string()]);
        assert_eq!(expected_require_set, result.require_paths);
    }

    #[test]
    fn test_import_call() {
        let result = run("import('foo')");
        let expected_import_paths = HashSet::from(["foo".to_string()]);
        assert_eq!(expected_import_paths, result.import_paths);
    }

    #[test]
    fn test_nested_import_call() {
        let result = run("import(import('import_subrequire').default + '/parent')");
        let expected_import_paths = HashSet::from(["import_subrequire".to_string()]);
        assert_eq!(expected_import_paths, result.import_paths);
    }

    #[test]
    fn test_require_shadowing() {
        // require at outer scope is global; require shadowed inside an IIFE is local.
        let result = run(r#"
            require("foo");
            (function() {
              const require = console.log;
              require("bar");
            })();
            require("original")
        "#);
        let expected_require_set = HashSet::from(["foo".to_string(), "original".to_string()]);
        assert_eq!(expected_require_set, result.require_paths);
    }

    #[test]
    fn test_imports() {
        let result = run("import foo from './bar';");
        let expected_import_map =
            HashMap::from([("./bar".to_string(), HashSet::from(["default".to_string()]))]);
        assert_eq!(expected_import_map, result.imports_map);
    }

    #[test]
    fn trest_import_with_satisfies() {
        let result = run(r#"
            import foo from './bar';
            foo satisfies never;
        "#);
        let expected_import_map =
            HashMap::from([("./bar".to_string(), HashSet::from(["default".to_string()]))]);
        assert_eq!(expected_import_map, result.imports_map);
    }

    #[test]
    fn test_imports_specifiers() {
        let result = run("import {foo, bar} from './bar';");
        let expected_import_map = HashMap::from([(
            "./bar".to_string(),
            HashSet::from(["foo".to_string(), "bar".to_string()]),
        )]);
        assert_eq!(expected_import_map, result.imports_map);
    }

    #[test]
    fn test_require_redefinition() {
        // `var require` is hoisted to the top of the scope; both call sites resolve
        // to the local symbol, so neither is treated as the global require.
        let result = run(r#"
            require('before_definition')
            var require = function(){}
            require('after_definition')
        "#);
        assert!(
            result.require_paths.is_empty(),
            "expected no global require calls, got {:?}",
            result.require_paths
        );
    }

    #[test]
    fn test_require_inside_call_expr() {
        let result = run(r#"
            (function otherFunction() {})(require('arg_subrequire'))
            (require('callee_subrequire'))("foo")
        "#);
        let expected_require_set = HashSet::from([
            "arg_subrequire".to_string(),
            "callee_subrequire".to_string(),
        ]);
        assert_eq!(expected_require_set, result.require_paths);
    }

    #[test]
    fn test_require_inside_require() {
        let result = run(r#"require(require('require_subrequire').default + '/parent')"#);
        let expected_require_set = HashSet::from(["require_subrequire".to_string()]);
        assert_eq!(expected_require_set, result.require_paths);
    }
}
