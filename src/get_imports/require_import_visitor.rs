use std::collections::{HashMap, HashSet};

use swc_core::visit::swc_ecma_ast;
use swc_ecma_ast::{
    AssignExpr, CallExpr, Callee, Id, ImportDecl, Lit, ModuleExportName,
    TsImportEqualsDecl, VarDecl,
};
use swc_ecmascript::visit::{Visit, VisitWith};
#[derive(Debug)]
pub struct ImportPathCheckerVisitor {
    pub require_paths: HashSet<String>,
    pub import_paths: HashSet<String>,
    pub imports_map: HashMap<String, HashSet<String>>,
    pub require_identifiers: HashSet<Id>,
}

impl ImportPathCheckerVisitor {
    pub fn new() -> Self {
        Self {
            require_paths: HashSet::new(),
            import_paths: HashSet::new(),
            imports_map: HashMap::new(),
            require_identifiers: HashSet::new(),
        }
    }
}

impl Visit for ImportPathCheckerVisitor {
    fn visit_var_decl(&mut self, decl: &VarDecl) {
        decl.visit_children_with(self);
        decl.decls.iter().for_each(|decl| match &decl.name {
            swc_ecma_ast::Pat::Ident(ident) => {
                if ident.sym.to_string() == "require".to_string() {
                    self.require_identifiers.insert(ident.to_id());
                }
            }
            _ => {}
        })
    }

    fn visit_assign_expr(&mut self, expr: &AssignExpr) {
        expr.visit_children_with(self);
        if let Some(ident) = expr.right.as_ident() {
            dbg!(ident.sym.to_string());
            if ident.sym.to_string() == "require".to_string() {
                if let Some(req_ident) = expr.left.as_ident() {
                    let a = req_ident.to_id();
                    dbg!(&a);
                    self.require_identifiers.insert(a);
                }
            }
        }

        if let Some(ident) = expr.left.as_ident() {
            if ident.sym.to_string() == "require" {

            }
        }
    }

    fn visit_ts_import_equals_decl(&mut self, decl: &TsImportEqualsDecl) {
        decl.visit_children_with(self);
        if let Some(module_ref) = decl.module_ref.as_ts_external_module_ref() {
            self.import_paths.insert(module_ref.expr.value.to_string());
        }
    }
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                if ident.sym.to_string() == "import" {
                    match extract_argument_value(expr) {
                        Some(import_path) => {
                            self.import_paths.insert(import_path);
                        }
                        None => return,
                    }
                }
                if ident.sym.to_string() == "require" {
                    if !self.require_identifiers.contains(&ident.to_id()) {
                        match extract_argument_value(expr) {
                            Some(import_path) => {
                                self.require_paths.insert(import_path);
                            }
                            None => return,
                        }
                    }
                }
            }
        }
    }

    fn visit_import_decl(&mut self, node: &ImportDecl) {
        node.visit_children_with(self);
        let source_path = node.src.value.to_string();
        if let Some(imported_names) = self.imports_map.get_mut(&source_path) {
            for spec in &node.specifiers {
                append_imported_names(spec, imported_names);
            }
        } else {
            let mut imported_names = HashSet::new();
            for spec in &node.specifiers {
                append_imported_names(spec, &mut imported_names);
            }
            self.imports_map.insert(source_path.clone(), imported_names);
        }
    }
}

fn append_imported_names(
    spec: &swc_ecma_ast::ImportSpecifier,
    imported_names: &mut HashSet<String>,
) {
    if let Some(named) = spec.as_named() {
        match &named.imported {
            Some(imported) => match imported {
                ModuleExportName::Ident(identifier) => {
                    imported_names.insert(identifier.sym.to_string());
                }
                ModuleExportName::Str(str_value) => {
                    imported_names.insert(str_value.value.to_string());
                }
            },
            None => {
                imported_names.insert(named.local.sym.to_string());
            }
        }
    }
    if let Some(default) = spec.as_default() {
        imported_names.insert(default.local.sym.to_string());
    }
}

fn extract_argument_value(expr: &CallExpr) -> Option<String> {
    let import_path = match expr.args.is_empty() {
        true => return None,
        false => expr.args.get(0),
    };
    if let Some(path) = import_path {
        if let Some(path_lit) = path.expr.as_lit() {
            match path_lit {
                Lit::Str(value) => {
                    return Some(value.value.to_string());
                    // self.require_paths.insert(value.value.to_string());
                }
                _ => return None,
            }
        }
    }
    None
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use swc_common::sync::Lrc;
    use swc_common::{FileName, SourceMap};
    use swc_ecma_parser::{Capturing, Parser};
    use swc_ecma_visit::visit_module;

    use crate::get_imports::create_lexer;

    use super::ImportPathCheckerVisitor;

    #[test]
    fn test_require_imports() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"require('hello-world')"#.to_string(),
        );

        let mut parser = create_test_parser(&fm);

        let mut visitor = ImportPathCheckerVisitor::new();
        let module = parser.parse_typescript_module().unwrap();

        visit_module(&mut visitor, &module);
        let expected_require_set = HashSet::from(["hello-world".to_string()]);
        assert_eq!(expected_require_set, visitor.require_paths);
    }
    #[test]
    fn test_require_shadowing() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"require("foo")
            (function() {
              const require = console.log
              require("bar")
            })()
            require("original")
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);

        let mut visitor = ImportPathCheckerVisitor::new();
        let module = parser.parse_typescript_module().unwrap();

        visit_module(&mut visitor, &module);
        let expected_require_set = HashSet::from(["foo".to_string(), "original".to_string()]);
        assert_eq!(expected_require_set, visitor.require_paths);
    }

    #[test]
    fn test_imports() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo from './bar';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);

        let mut visitor = ImportPathCheckerVisitor::new();
        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        let expected_import_map =
            HashMap::from([("./bar".to_string(), HashSet::from(["foo".to_string()]))]);

        assert_eq!(expected_import_map, visitor.imports_map);
    }

    #[test]
    fn test_imports_specifiers() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {foo, bar} from './bar';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);

        let mut visitor = ImportPathCheckerVisitor::new();
        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        let expected_import_map = HashMap::from([(
            "./bar".to_string(),
            HashSet::from(["foo".to_string(), "bar".to_string()]),
        )]);

        assert_eq!(expected_import_map, visitor.imports_map);
    }

    #[test]
    fn test_require_redefinition() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            require('before_definition')
            var require = function(){}
            require('after_definition')
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);

        let mut visitor = ImportPathCheckerVisitor::new();
        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        let expected_require_set = HashSet::from(["before_definition".to_string()]);

        assert_eq!(expected_require_set, visitor.require_paths);
    }

    fn create_test_parser<'a>(
        fm: &'a Lrc<swc_common::SourceFile>,
    ) -> Parser<Capturing<swc_ecma_parser::lexer::Lexer<'a, swc_ecma_parser::StringInput<'a>>>>
    {
        let lexer = create_lexer(fm);
        let capturing = Capturing::new(lexer);
        let parser = Parser::new_from(capturing);
        parser
    }
}
