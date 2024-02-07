use std::collections::{HashMap, HashSet};

use swc_core::ecma::{ast::Ident, visit::{Visit, VisitWith}};

#[derive(Debug, Default, Clone)]
pub struct ImportExtractor {
    // Hashmap that represents the item local name as key, and the item path as value.
    pub imported_items: HashMap<String, String>,
    pub exported_declarations: HashSet<String>,
    pub exported_types: HashSet<String>,
    pub types_used_in_exports: HashSet<String>,
    pub has_default_export: bool,
}

impl Visit for ImportExtractor {
    fn visit_module_item(&mut self, n: &swc_core::ecma::ast::ModuleItem) {
        n.visit_children_with(self);
        match n {
            swc_core::ecma::ast::ModuleItem::ModuleDecl(module) => {
                match module {
                    swc_core::ecma::ast::ModuleDecl::Import(import) => {
                        import.specifiers.iter().for_each(|specifier| {
                            match specifier {
                                swc_core::ecma::ast::ImportSpecifier::Named(named) => {
                                    if let Some(imported) = &named.imported {
                                        match imported {
                                            swc_core::ecma::ast::ModuleExportName::Ident(ident) => {
                                                self.imported_items.insert(ident.sym.to_string(), import.src.value.to_string());
                                            },
                                            _ => {},
                                        }
                                    } else {
                                        self.imported_items.insert(named.local.sym.to_string(), import.src.value.to_string());
                                    }
                                },
                                swc_core::ecma::ast::ImportSpecifier::Default(_) => {
                                    self.imported_items.insert("default".to_string(), import.src.value.to_string());
                                },
                                swc_core::ecma::ast::ImportSpecifier::Namespace(_) => {
                                    self.imported_items.insert("*".to_string(), import.src.value.to_string());
                                },
                            }
                        });
                    },
                    swc_core::ecma::ast::ModuleDecl::ExportDecl(_) => {},
                    swc_core::ecma::ast::ModuleDecl::ExportNamed(_) => {},
                    swc_core::ecma::ast::ModuleDecl::ExportDefaultDecl(_) => {},
                    swc_core::ecma::ast::ModuleDecl::ExportDefaultExpr(_) => {},
                    swc_core::ecma::ast::ModuleDecl::ExportAll(_) => {},
                    swc_core::ecma::ast::ModuleDecl::TsImportEquals(_) => {},
                    swc_core::ecma::ast::ModuleDecl::TsExportAssignment(_) => {},
                    swc_core::ecma::ast::ModuleDecl::TsNamespaceExport(_) => {},
                }
            },
            swc_core::ecma::ast::ModuleItem::Stmt(stmnt) => {
                
            },
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};
    use std::iter::FromIterator;
    use std::sync::Arc;

    use swc_core::common::comments::{Comments, SingleThreadedComments};
    use swc_core::common::{FileName, SourceFile, SourceMap};
    use swc_core::ecma::visit::visit_module;
    use swc_ecma_parser::lexer::Lexer;
    use swc_ecma_parser::{Capturing, Parser};

    use crate::get_imports::create_lexer;
    use crate::ts_shaker::syntax_scanner::ImportExtractor;

    fn create_test_parser<'a>(
        fm: &'a Arc<SourceFile>,
        comments: Option<&'a dyn Comments>,
    ) -> Parser<Capturing<Lexer<'a>>> {
        let lexer = create_lexer(fm, comments);
        let capturing = Capturing::new(lexer);
        let parser = Parser::new_from(capturing);
        parser
    }

    #[test]
    fn test_allowed_unused_export_named() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                const foo = 1;
                export { foo }
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ImportExtractor::default();
        visit_module(&mut visitor, &module);
        // assert!(
        //     visitor.exported_ids.iter().all(|exps| exps.allow_unused),
        //     "Check for every exported item to be allowed_unused = true failed"
        // );
    }

}