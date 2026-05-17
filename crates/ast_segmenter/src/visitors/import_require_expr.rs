use swc_ecma_ast::{
    AssignPatProp, BindingIdent, CallExpr, Callee, Expr, ExprOrSpread, Ident, IdentName,
    KeyValuePatProp, Lit, MemberExpr, MemberProp, Pat, PropName,
};
use swc_ecma_visit::{Visit, VisitWith};

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
    // Checks if the current call expr is one of the supported import() or require() calls
    // and if it is, updates this data structure with the import path and the names that are
    // being imported.
    //
    // Note that this is not actually an implementation of swc's Visit() because we want to
    // perform all the visits in a single pass, in order to avoid cache-misses caused by multiple
    // traverses over the AST nodes, which may be distributed across the heap.
    pub fn scan_call_expr(&mut self, expr: &CallExpr) {
        match expr {
            // import()
            CallExpr {
                callee: Callee::Import(_),
                args: ref import_args,
                ..
            } => {
                if let Some(import_path) = args_as_import(import_args) {
                    self.imported_paths.insert(import_path, Symbol::Namespace);
                }
            }
            // require()
            CallExpr {
                callee: Callee::Expr(box Expr::Ident(ident)),
                args: ref import_args,
                ..
            } => {
                if ident.sym == "require" {
                    if let Some(import_path) = args_as_import(import_args) {
                        self.require_paths.insert(import_path, Symbol::Default);
                    }
                }
            }
            // import().then(({name1, name2, name3}) => {...})
            CallExpr {
                callee:
                    Callee::Expr(box Expr::Member(MemberExpr {
                        // import expr
                        obj:
                            box Expr::Call(CallExpr {
                                callee: Callee::Import(_),
                                args: import_args,
                                ..
                            }),
                        prop: MemberProp::Ident(then_prop),
                        ..
                    })),
                args: ref args,
                ..
            } => {
                if then_prop.sym != "then" {
                    return;
                }
                // the contents of the import(<this stuff>) call
                let imported_path = match args_as_import(import_args) {
                    Some(path) => path,
                    None => return,
                };

                // args in .then((<args>) => {..}) or .then(function (<args>) {..})
                let then_arg_obj_pattern = match args.first() {
                    Some(arg) => match extract_generic_function_def_first_arg(&arg.expr) {
                        Some(Pat::Object(obj_pat)) => obj_pat,
                        _ => return,
                    },
                    None => return,
                };

                // extract names from the object binding pattern
                let obj_names =
                    then_arg_obj_pattern
                        .props
                        .iter()
                        .filter_map(|prop| -> Option<Symbol> {
                            match prop {
                                swc_ecma_ast::ObjectPatProp::KeyValue(KeyValuePatProp {
                                    key:
                                        PropName::Ident(IdentName {
                                            sym: ref ident_sym, ..
                                        }),
                                    ..
                                })
                                | swc_ecma_ast::ObjectPatProp::Assign(AssignPatProp {
                                    key:
                                        BindingIdent {
                                            id:
                                                Ident {
                                                    sym: ref ident_sym, ..
                                                },
                                            ..
                                        },
                                    ..
                                }) => Some(Symbol::Named(Name::from(ident_sym.as_ref()))),
                                _ => None,
                            }
                        });

                // store the names (removing any Namespace entry that was added
                // by visit_children_with processing the inner import() call)
                let entry = self.imported_paths.entry(imported_path).or_default();
                entry.remove(&Symbol::Namespace);
                for name in obj_names {
                    entry.insert(name);
                }
            }
            _ => {}
        }
    }
}

fn extract_generic_function_def_first_arg(expr: &Expr) -> Option<&Pat> {
    if let Expr::Arrow(arrow) = expr {
        return arrow.params.first();
    }
    if let Expr::Fn(fn_expr) = expr {
        return fn_expr.function.params.first().map(|param| &param.pat);
    }
    None
}

fn args_as_import(args: &Vec<ExprOrSpread>) -> Option<String> {
    let import_path = match args.is_empty() {
        true => return None,
        false => args.first(),
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

impl Visit for ImportsAndRequires {
    fn visit_call_expr(&mut self, call_expr: &CallExpr) {
        call_expr.visit_children_with(self);
        self.scan_call_expr(call_expr);
    }
}

pub fn find_imports_and_requires<TNode>(ast_node: &TNode) -> ImportsAndRequires
where
    TNode: for<'a> VisitWith<ImportsAndRequires>,
{
    let mut visitor = ImportsAndRequires::default();
    ast_node.visit_with(&mut visitor);
    visitor
}

#[cfg(test)]
mod test {
    use crate::raw_module_deps::Symbol;

    use super::ImportsAndRequires;
    use ahashmap::AHashMap;

    use test_tmpdir::amap2;

    fn test_discovers_import_expr(
        source: &str,
        expected_imported_paths: AHashMap<&str, Vec<Symbol>>,
        expected_require_paths: AHashMap<&str, Vec<Symbol>>,
    ) {
        let mut visitor = ImportsAndRequires {
            imported_paths: Default::default(),
            require_paths: Default::default(),
        };
        swc_utils_parse::parse_and_visit(source, &mut visitor).unwrap();

        assert_eq!(
            visitor.imported_paths.names(),
            expected_imported_paths
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().cloned().collect()))
                .collect(),
        );

        assert_eq!(
            visitor.require_paths.names(),
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
