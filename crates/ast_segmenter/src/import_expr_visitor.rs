use ahashmap::{AHashMap, AHashSet};
use logger_srcfile::SrcFileLogger;
use swc_ecma_ast::{
    AssignPatProp, BindingIdent, CallExpr, Callee, Expr, ExprOrSpread, Ident, IdentName,
    KeyValuePatProp, Lit, MemberExpr, MemberProp, Pat, PropName,
};
use swc_ecma_visit::{Visit, VisitWith};

struct NameSet<K, V> {
    names: AHashMap<K, AHashSet<V>>,
}
impl<K, V> NameSet<K, V> {
    fn new() -> Self {
        Self {
            names: Default::default(),
        }
    }
}
impl<K: Eq + std::hash::Hash, V: Eq + std::hash::Hash> NameSet<K, V> {
    pub fn insert(&mut self, key: K, value: V) {
        self.names.entry(key).or_default().insert(value);
    }

    pub fn insert_all(&mut self, key: K, values: impl IntoIterator<Item = V>) {
        let entry = self.names.entry(key).or_default();
        for value in values {
            entry.insert(value);
        }
    }

    pub fn insert_nameless(&mut self, key: K) {
        self.names.entry(key).or_default();
    }
}
impl<K: Default, V: Default> Default for NameSet<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct ImportsAndRequires {
    imported_paths: NameSet<String, String>,
    require_paths: NameSet<String, String>,
}

fn is_import_expr(call_expr: &CallExpr) -> bool {
    if let Callee::Import(_) = &call_expr.callee {
        return true;
    }
    if let Callee::Expr(callee) = &call_expr.callee {
        if let Some(ident) = callee.as_ident() {
            if ident.sym == "require" {
                return true;
            }
        }
    }
    false
}

impl Visit for ImportsAndRequires {
    // import('foo')
    // or
    // require('foo')
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        match expr {
            // import()
            CallExpr {
                callee: Callee::Import(_),
                args: ref import_args,
                ..
            } => {
                if let Some(import_path) = args_as_import(import_args) {
                    self.imported_paths.insert_nameless(import_path);
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
                        self.require_paths.insert_nameless(import_path);
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
                        .filter_map(|prop| -> Option<String> {
                            println!("arg prop: {:?}", prop);

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
                                }) => Some(ident_sym.to_string()),
                                _ => None,
                            }
                        });

                // store the names
                self.imported_paths.insert_all(imported_path, obj_names);
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

pub fn find_imports_and_requires<TLogger, TNode>(ast_node: &TNode) -> ImportsAndRequires
where
    TLogger: SrcFileLogger,
    TNode: for<'a> VisitWith<ImportsAndRequires>,
{
    let mut visitor = ImportsAndRequires::default();
    ast_node.visit_with(&mut visitor);
    visitor
}

#[cfg(test)]
mod test {
    use super::ImportsAndRequires;
    use ahashmap::AHashMap;

    use test_tmpdir::amap2;

    fn test_discovers_import_expr(
        source: &str,
        expected_imported_paths: AHashMap<&str, Vec<&str>>,
        expected_require_paths: AHashMap<&str, Vec<&str>>,
    ) {
        let mut visitor = ImportsAndRequires {
            imported_paths: Default::default(),
            require_paths: Default::default(),
        };
        swc_utils_parse::parse_and_visit(source, &mut visitor).unwrap();

        assert_eq!(
            visitor.imported_paths.names,
            expected_imported_paths
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        );

        assert_eq!(
            visitor.require_paths.names,
            expected_require_paths
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        );
    }

    #[test]
    fn test_basic_import() {
        test_discovers_import_expr(
            "import('foo')",
            amap2![
                "foo" => vec![]
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
                "foo" => vec![]
            ],
        );
    }

    #[test]
    fn test_import_expr_deep() {
        test_discovers_import_expr(
            "if (true) { import('foo') } else { require('bar') }",
            amap2![
                "foo" => vec![]
            ],
            amap2![
                "bar" => vec![]
            ],
        );
    }

    #[test]
    fn test_import_expr_extracts_names_arrow() {
        test_discovers_import_expr(
            "import('foo').then(({a,b,c}) => { console.log(a,b,c) })",
            amap2![
                "foo" => vec!["a","b","c"]
            ],
            Default::default(),
        );
    }

    #[test]
    fn test_import_expr_extracts_names_noarrow() {
        test_discovers_import_expr(
            "import('foo').then(function myfunc({a,b,c}) { console.log(a,b,c) })",
            amap2![
                "foo" => vec!["a","b","c"]
            ],
            Default::default(),
        );
    }
}
