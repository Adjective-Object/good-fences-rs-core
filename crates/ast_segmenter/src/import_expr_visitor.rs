use ahashmap::AHashSet;
use swc_ecma_ast::{CallExpr, Callee, Lit};
use swc_ecma_visit::{Visit, VisitWith};

pub struct ImportRequireExprVisitor {
    imported_paths: AHashSet<String>,
    require_paths: AHashSet<String>,
}

impl Visit for ImportRequireExprVisitor {
    // import('foo')
    // or
    // require('foo')
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        if let Callee::Import(_) = &expr.callee {
            match extract_argument_value(expr) {
                Some(import_path) => {
                    self.imported_paths.insert(import_path);
                }
                None => return,
            }
        }
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                if ident.sym == "require" {
                    if let Some(import_path) = extract_argument_value(expr) {
                        self.require_paths.insert(import_path);
                    }
                }
            }
        }
    }
}

fn extract_argument_value(expr: &CallExpr) -> Option<String> {
    let import_path = match expr.args.is_empty() {
        true => return None,
        false => expr.args.first(),
    };
    if let Some(path) = import_path {
        if let Some(path_lit) = path.expr.as_lit() {
            match path_lit {
                Lit::Str(value) => {
                    return Some(value.value.to_string());
                }
                _ => return None,
            }
        }
    }
    None
}

#[cfg(test)]
mod test {
    use super::ImportRequireExprVisitor;

    fn test_discovers_import_expr(
        source: &str,
        expected_imported_paths: Vec<&str>,
        expected_require_paths: Vec<&str>,
    ) {
        let mut visitor = ImportRequireExprVisitor {
            imported_paths: Default::default(),
            require_paths: Default::default(),
        };
        swc_utils_parse::parse_and_visit(source, &mut visitor).unwrap();

        assert_eq!(
            visitor.imported_paths,
            expected_imported_paths
                .into_iter()
                .map(|s| s.to_string())
                .collect()
        );
        assert_eq!(
            visitor.require_paths,
            expected_require_paths
                .into_iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    #[test]
    fn test_basic_import() {
        test_discovers_import_expr("import('foo')", vec!["foo"], vec![]);
    }

    #[test]
    fn test_basic_require() {
        test_discovers_import_expr("require('foo')", vec![], vec!["foo"]);
    }

    #[test]
    fn test_import_expr_deep() {
        test_discovers_import_expr(
            "if (true) { import('foo') } else { require('bar') }",
            vec!["foo"],
            vec!["bar"],
        );
    }
}
