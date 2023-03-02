use std::{
    collections::{HashMap, HashSet},
    iter::FromIterator,
};
use swc_ecma_ast::{
    BindingIdent, CallExpr, Callee, ExportAll, ExportDecl, ExportDefaultDecl, ExportDefaultExpr,
    ExportSpecifier, Id, ImportDecl, Lit, ModuleExportName, NamedExport, Pat, TsImportEqualsDecl,
};
use swc_ecmascript::visit::{Visit, VisitWith};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ExportedItem {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl From<&ImportedItem> for ExportedItem {
    fn from(i: &ImportedItem) -> Self {
        match i {
            ImportedItem::Named(named) => ExportedItem::Named(named.clone()),
            ImportedItem::Default => ExportedItem::Default,
            ImportedItem::Namespace => ExportedItem::Namespace,
            ImportedItem::ExecutionOnly => ExportedItem::ExecutionOnly,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ImportedItem {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl From<&ExportedItem> for ImportedItem {
    fn from(e: &ExportedItem) -> Self {
        match e {
            ExportedItem::Named(name) => ImportedItem::Named(name.clone()),
            ExportedItem::Default => ImportedItem::Default,
            ExportedItem::Namespace => ImportedItem::Namespace,
            ExportedItem::ExecutionOnly => ImportedItem::ExecutionOnly,
        }
    }
}

#[derive(Debug)]
pub struct UnusedFinderVisitor {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_ids_path_name: HashMap<String, HashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: HashSet<String>,
    // import('./foo') and import './foo' generates ["./foo"]
    pub imported_paths: HashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: HashMap<String, HashSet<ExportedItem>>,
    pub exported_ids: HashSet<ExportedItem>,
    // exported from this file
    // const foo = require('foo') generates ["foo"]
    require_identifiers: HashSet<Id>,
}

impl UnusedFinderVisitor {
    pub fn new() -> Self {
        Self {
            imported_ids_path_name: HashMap::new(),
            require_paths: HashSet::new(),
            imported_paths: HashSet::new(),
            export_from_ids: HashMap::new(),
            require_identifiers: HashSet::new(),
            exported_ids: HashSet::new(),
        }
    }

    /**
     * Extracts information from each specifier imported in source to treat it as an string
     * Supported sytax list:
     * - `export { foo as bar } from 'foo'`
     * - `export { default as foo } from 'foo'`
     * - `export { foo } from 'foo'`
     */
    fn handle_export_from_specifiers(&mut self, export: &NamedExport, source: &swc_ecma_ast::Str) {
        let mut specifiers: Vec<ExportedItem> = export
            .specifiers
            .iter()
            .filter_map(|spec| -> Option<ExportedItem> {
                if spec.is_namespace() {
                    // export * as foo from 'foo;
                    return Some(ExportedItem::Namespace);
                }
                if let Some(named) = spec.as_named() {
                    // export { foo } from 'foo'
                    if let ModuleExportName::Ident(ident) = &named.orig {
                        // export { default as foo } from 'foo'
                        if ident.sym.to_string() == "default" {
                            return Some(ExportedItem::Default);
                        }
                        // export { foo } from 'foo'
                        return Some(ExportedItem::Named(ident.sym.to_string()));
                    }
                }
                return None;
            })
            .collect();
        if let Some(entry) = self.export_from_ids.get_mut(&source.value.to_string()) {
            specifiers.drain(0..).for_each(|s| {
                entry.insert(s);
            })
        } else {
            self.export_from_ids
                .insert(source.value.to_string(), HashSet::from_iter(specifiers));
        }
    }

    /**
     * Extracts information from the ExportSpecifier to construct the map of exported items with its values.
     * supports `export { foo }` and aliased `export { foo as bar }`
     */
    fn handle_export_named_specifiers(&mut self, specs: &Vec<ExportSpecifier>) {
        specs.iter().for_each(|specifier| match specifier {
            ExportSpecifier::Named(named) => {
                // Handles `export { foo as bar }`
                if let Some(exported) = &named.exported {
                    if let ModuleExportName::Ident(id) = exported {
                        let sym = id.sym.to_string();
                        // export { foo as default }
                        if sym == "default" {
                            self.exported_ids.insert(ExportedItem::Default);
                        } else {
                            self.exported_ids
                                .insert(ExportedItem::Named(id.sym.to_string()));
                        }
                    }
                } else if let ModuleExportName::Ident(id) = &named.orig {
                    // handles `export { foo }`
                    self.exported_ids
                        .insert(ExportedItem::Named(id.sym.to_string()));
                }
            }
            _ => {}
        });
    }
}

impl Visit for UnusedFinderVisitor {
    // Handles `export default foo`
    fn visit_export_default_expr(&mut self, _: &ExportDefaultExpr) {
        self.exported_ids.insert(ExportedItem::Default);
    }

    /**
     * Handles scenarios where `export default` has an inline declaration, e.g. `export default class Foo {}` or `export default function foo() {}`
     */
    fn visit_export_default_decl(&mut self, _: &ExportDefaultDecl) {
        self.exported_ids.insert(ExportedItem::Default);
    }

    // Handles scenarios `export` has an inline declaration, e.g. `export const foo = 1` or `export class Foo {}`
    fn visit_export_decl(&mut self, export: &ExportDecl) {
        match &export.decl {
            swc_ecma_ast::Decl::Class(decl) => {
                // export class Foo {}
                self.exported_ids
                    .insert(ExportedItem::Named(decl.ident.sym.to_string()));
            }
            swc_ecma_ast::Decl::Fn(decl) => {
                // export function foo() {}
                self.exported_ids
                    .insert(ExportedItem::Named(decl.ident.sym.to_string()));
            }
            swc_ecma_ast::Decl::Var(decl) => {
                // export const foo = 1;
                if let Some(d) = decl.decls.first() {
                    if let Pat::Ident(ident) = &d.name {
                        self.exported_ids
                            .insert(ExportedItem::Named(ident.sym.to_string()));
                    }
                }
                // self.exported_ids.insert(ExportedItem::Named(decl.ident.sym.to_string()));
            }
            swc_ecma_ast::Decl::TsInterface(decl) => {
                // export interface Foo {}
                self.exported_ids
                    .insert(ExportedItem::Named(decl.id.sym.to_string()));
            }
            swc_ecma_ast::Decl::TsTypeAlias(decl) => {
                // export type foo = string
                self.exported_ids
                    .insert(ExportedItem::Named(decl.id.sym.to_string()));
            }
            swc_ecma_ast::Decl::TsEnum(decl) => {
                // export enum Foo { foo, bar }
                self.exported_ids
                    .insert(ExportedItem::Named(decl.id.sym.to_string()));
            }
            swc_ecma_ast::Decl::TsModule(decl) => {
                dbg!(decl);
                // self.exported_ids.insert(ExportedItem::Named(decl.id.as_ident().unwrap().to_string()));
            }
        }
    }

    // `export * from './foo'`; // TODO allow recursive import resolution
    fn visit_export_all(&mut self, export: &ExportAll) {
        export.visit_children_with(self);

        let source = export.src.value.to_string();

        self.export_from_ids
            .insert(source, HashSet::from_iter(vec![ExportedItem::Namespace]));
    }

    // export {foo} from './foo';
    fn visit_named_export(&mut self, export: &NamedExport) {
        export.visit_children_with(self);

        if let Some(source) = &export.src {
            // In case we find `'./foo'` in `export { foo } from './foo'`
            self.handle_export_from_specifiers(export, source);
        } else {
            self.handle_export_named_specifiers(&export.specifiers);
        }
    }

    // const foo = require; // <- Binding
    // const p = foo('./path')
    fn visit_binding_ident(&mut self, binding: &BindingIdent) {
        binding.visit_children_with(self);
        if binding.sym.to_string() == "require".to_string() {
            self.require_identifiers.insert(binding.id.to_id());
        }
    }

    // import foo = require('./foo')
    fn visit_ts_import_equals_decl(&mut self, decl: &TsImportEqualsDecl) {
        decl.visit_children_with(self);
        if let Some(module_ref) = decl.module_ref.as_ts_external_module_ref() {
            self.imported_paths
                .insert(module_ref.expr.value.to_string());
        }
    }

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

    // import foo from './foo';
    fn visit_import_decl(&mut self, import: &ImportDecl) {
        import.visit_children_with(self);

        let src = import.src.value.to_string();
        // import './foo';
        if import.specifiers.is_empty() {
            self.imported_paths.insert(src);
            return;
        }
        // import .. from ..
        let mut specifiers: Vec<ExportedItem> = import
            .specifiers
            .iter()
            .filter_map(|spec| -> Option<ExportedItem> {
                match spec {
                    swc_ecma_ast::ImportSpecifier::Named(named) => {
                        match &named.imported {
                            Some(module_name) => {
                                // import { foo as bar } from './foo'
                                match module_name {
                                    ModuleExportName::Ident(ident) => {
                                        // sym_str = foo in `import { foo as bar } from './foo'`
                                        let sym_str = ident.sym.to_string();
                                        dbg!(&sym_str);
                                        if sym_str == "default" {
                                            return Some(ExportedItem::Default);
                                        }
                                        return Some(ExportedItem::Named(sym_str));
                                    }
                                    ModuleExportName::Str(s) => {
                                        return Some(ExportedItem::Named(s.value.to_string()))
                                    }
                                }
                            }
                            None => {
                                // import { foo } from './foo'
                                return Some(ExportedItem::Named(named.local.sym.to_string()));
                            }
                        }
                    }
                    swc_ecma_ast::ImportSpecifier::Default(_) => {
                        // import foo from 'foo'
                        return Some(ExportedItem::Default);
                    }
                    swc_ecma_ast::ImportSpecifier::Namespace(_) => {
                        // import * as foo from 'foo'
                        return Some(ExportedItem::Namespace);
                    }
                }
            })
            .collect();

        if let Some(entry) = self.imported_ids_path_name.get_mut(&src) {
            specifiers.drain(0..).for_each(|s| {
                entry.insert(ImportedItem::from(&s));
            });
        } else {
            self.imported_ids_path_name.insert(
                src,
                HashSet::from_iter(specifiers.iter().map(|s| ImportedItem::from(s))),
            );
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
    use std::iter::FromIterator;
    use swc_common::sync::Lrc;
    use swc_common::{FileName, SourceMap};
    use swc_ecma_parser::{Capturing, Parser};
    use swc_ecma_visit::visit_module;

    use crate::get_imports::create_lexer;
    use crate::unused_finder::node_visitor::{ExportedItem, ImportedItem};

    use super::UnusedFinderVisitor;

    #[test]
    fn test_export_named() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export { foo }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("foo".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_named_as_bar() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export { foo as bar }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("bar".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }
    #[test]
    fn test_export_default() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export default foo;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_expor_type_as_default() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            interface Foo {
                bar: boolean;
            }
            export type { Foo as default };
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_default_execution() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            function foo() { return 1; }
            export default foo();
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_default_class() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export default class Foo {}
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_const() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export const foo = 1;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("foo".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_from() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export { foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ExportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ExportedItem::Named("foo".to_owned())]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_default_from() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export { default as foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ExportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ExportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_star_from() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export * from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ExportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ExportedItem::Namespace]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_import_default() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Named("foo".to_owned())]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier_with_alias() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {foo as bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Named("foo".to_owned())]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_default_with_alias() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {default as foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_default_and_specifier() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo, {bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![
                ImportedItem::Default,
                ImportedItem::Named("bar".to_owned()),
            ]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_star() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import * as foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Namespace]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_require() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = require('./foo');
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.require_paths
        );
    }

    #[test]
    fn test_import_equals() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo = require('./foo')
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.imported_paths
        );
    }

    #[test]
    fn test_import_statement() {
        let cm = Lrc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import './foo'
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm);
        let mut visitor = UnusedFinderVisitor::new();

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.imported_paths
        );
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
