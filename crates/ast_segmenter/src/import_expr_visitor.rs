use ahashmap::{AHashMap, AHashSet};
use logger_srcfile::SrcFileLogger;
use swc_ecma_ast::{CallExpr, Callee, Lit};
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
    imported_paths: NameSet<String, Option<String>>,
    require_paths: NameSet<String, Option<String>>,
}

impl Visit for ImportsAndRequires {
    // import('foo')
    // or
    // require('foo')
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        if let Callee::Import(_) = &expr.callee {
            match extract_argument_value(expr) {
                Some(import_path) => {
                    self.imported_paths.insert_nameless(import_path);
                }
                None => return,
            }
        }
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                if ident.sym == "require" {
                    if let Some(import_path) = extract_argument_value(expr) {
                        self.require_paths.insert_nameless(import_path);
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
                .map(|(k, v)| (
                    k.to_string(),
                    v.iter().map(|s| Some(s.to_string())).collect()
                ))
                .collect(),
        );

        assert_eq!(
            visitor.require_paths.names,
            expected_require_paths
                .iter()
                .map(|(k, v)| (
                    k.to_string(),
                    v.iter().map(|s| Some(s.to_string())).collect()
                ))
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
}
