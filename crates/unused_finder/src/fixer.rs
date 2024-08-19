use std::collections::HashSet;

use swc_core::ecma::{
    ast::{ExportNamedSpecifier, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem},
    visit::Fold,
};

// Fixer that will remove any unused exports matching
// the names in its `unused_exports`.
pub struct UnusedFixer<'a> {
    unused_exports: &'a HashSet<String>,
}

impl<'a> UnusedFixer<'a> {
    pub fn new(unused_exports: &'a HashSet<String>) -> Self {
        Self { unused_exports }
    }
}

impl<'a> Fold for UnusedFixer<'a> {
    fn fold_module_items(&mut self, nodes: Vec<ModuleItem>) -> Vec<ModuleItem> {
        // look unused `export const x = ` and `export let x =` assignments.
        // If they have a single binding, or all bindings are unused replace the statement with
        // a non-exported `const x =` or `let x =`.
        //
        // If they have multiple bindings, and only some are unused, break the binding
        // into multiple bindings, and remove the `export` keyword from the unused ones.
        let mut next_nodes: Option<Vec<ModuleItem>> = None;
        for (i, node) in nodes.iter().enumerate() {
            match node {
                // export { a, b, c}
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named_export)) => {
                    let (unused, used): (Vec<&ExportSpecifier>, Vec<&ExportSpecifier>) =
                        named_export
                            .specifiers
                            .iter()
                            .partition(|specifier| match specifier {
                                ExportSpecifier::Named(ExportNamedSpecifier { orig, .. }) => {
                                    let exported_name_str = match orig {
                                        ModuleExportName::Ident(ident) => &ident.sym.to_string(),
                                        ModuleExportName::Str(str_) => &str_.value.to_string(),
                                    };
                                    self.unused_exports.contains(exported_name_str)
                                }
                                ExportSpecifier::Default(_) => {
                                    self.unused_exports.contains("default")
                                }
                                _ => false,
                            });
                    match (unused.len(), used.len()) {
                        // no unused exports, do nothing
                        (0, _) => {}
                        // no used exports, remove the entire export statement
                        (_, 0) => {
                            let cur_nodes = next_nodes.unwrap_or_else(|| nodes[0..i].to_vec());
                            next_nodes = Some(cur_nodes);
                        }
                        // some used, some unused exports, remove the unused ones
                        (_, _) => {
                            let mut cur_nodes = next_nodes.unwrap_or_else(|| nodes[0..i].to_vec());
                            let mut copy = named_export.clone();
                            copy.specifiers = used.into_iter().map(|f| f.clone()).collect();
                            cur_nodes.push(ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(copy)));

                            next_nodes = Some(cur_nodes)
                        }
                    }
                }
                _ => {}
            }
        }

        next_nodes.unwrap_or_else(|| nodes)
    }
}

#[cfg(test)]
mod test {
    use maplit::hashset;
    use swc::PrintArgs;
    use swc_core::common::comments::SingleThreadedComments;
    use swc_core::ecma::visit::FoldWith;
    use swc_utils::{normalise_src, parse_ecma_src_comments, print_ast};

    use super::UnusedFixer;

    #[test]
    fn test_export_name() {
        let comments = SingleThreadedComments::default();
        let (cm, module) = parse_ecma_src_comments(
            "test.ts",
            r#"
            const used_1 = 1;
            const used_2 = 1;
            const unused = 2;
            // "unused" should get removed
            export { unused, used_1, used_2 };
        "#,
            Some(&comments),
        );
        let names = hashset!["unused".to_string()];
        let mut unused_fixer = UnusedFixer::new(&names);
        let result = module.fold_with(&mut unused_fixer);

        assert_eq!(
            print_ast(
                &cm,
                &result,
                PrintArgs {
                    comments: Some(&comments),
                    ..Default::default()
                }
            ),
            normalise_src(
                r#"
                const used_1 = 1;
                const used_2 = 1;
                const unused = 2;
                // "unused" should get removed
                export { used_1, used_2 };;;; /* ;) */
                "#,
                PrintArgs {
                    ..Default::default()
                }
            )
        );
    }
}
