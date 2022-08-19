use std::collections::HashSet;

use swc_ecma_ast::{CallExpr, Callee, Ident, Lit};
use swc_ecmascript::visit::{as_folder, Folder, VisitMut, VisitMutWith};
#[derive(Debug)]
pub struct RequireImportVisitor {
    require_paths: HashSet<String>,
    import_paths: HashSet<String>,
}

impl RequireImportVisitor {}

pub fn node_visitor() -> Folder<RequireImportVisitor> {
    let mut visitor = RequireImportVisitor {
        require_paths: HashSet::new(),
        import_paths: HashSet::new(),
    };
    as_folder(visitor)
}

impl VisitMut for RequireImportVisitor {
    fn visit_mut_call_expr(&mut self, expr: &mut CallExpr) {
        expr.visit_mut_children_with(self);
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                println!("call expr test {}", ident.sym.to_string());
                if ident.sym.to_string() == "import" {
                    match extract_argument_value(expr) {
                        Some(import_path) => {
                            self.import_paths.insert(import_path);
                        }
                        None => return,
                    }
                }
                if ident.sym.to_string() == "require" {
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
    #[test]
    fn test_node_visitor_require() {
        // const filename = "tests/good_fences_integration/src/requireImportTest.ts";

    }
    #[test]
    fn test_node_visitor_import() {

    }
}
