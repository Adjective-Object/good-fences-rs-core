#[cfg(test)]
mod test {
    use std::sync::Arc;

    use ahashmap::{AHashMap, AHashSet};
    use swc_common::comments::{Comments, SingleThreadedComments};
    use swc_common::{FileName, SourceFile, SourceMap};
    use swc_ecma_parser::lexer::Lexer;
    use swc_ecma_parser::{Capturing, Parser};
    use swc_ecma_visit::VisitWith;

    use crate::parse::{ExportedSymbol, ReExportedSymbol};
    use swc_utils::create_lexer;

    use crate::parse::exports_visitor::ExportsVisitor;
    use test_tmpdir::{amap, aset};

    fn create_test_parser<'a>(
        fm: &'a Arc<SourceFile>,
        comments: Option<&'a dyn Comments>,
    ) -> Parser<Capturing<Lexer<'a>>> {
        let lexer = create_lexer(fm, comments);
        let capturing = Capturing::new(lexer);

        Parser::new_from(capturing)
    }

    #[test]
    fn test_allowed_unused_export_named() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export { foo }
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        assert!(
            visitor.exported_ids.values().all(|exps| exps.allow_unused),
            "Check for every exported item to be allowed_unused = true failed"
        );
    }

    #[test]
    fn test_allowed_unused_export_named_as_bar() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export { foo as bar }
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);

        assert_eq!(visitor.exported_ids.len(), 1);
        assert!(visitor.exported_ids.iter().all(
            |(symbol, e)| e.allow_unused && *symbol == ExportedSymbol::Named("bar".to_string())
        ));
    }
    #[test]
    fn test_allowed_unused_export_default() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export default foo;
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = AHashSet::default();
        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_allowed_unused_export_kind_as_default() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                interface Foo {
                    bar: boolean;
                }
                // @ALLOW-UNUSED-EXPORT
                export type { Foo as default };
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        assert!(visitor
            .exported_ids
            .iter()
            .all(|(symbol, e)| e.allow_unused && *symbol == ExportedSymbol::Default));
    }

    #[test]
    fn test_allowed_unused_export_default_execution() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                function foo() { return 1; }
                // @ALLOW-UNUSED-EXPORT
                export default foo();
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = AHashSet::default();
        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_allowed_unused_export_default_class() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export default class Foo {}
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = AHashSet::default();
        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_allowed_unused_export_const() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export const foo = 1;
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);

        assert!(visitor.exported_ids.iter().all(|(_, e)| e.allow_unused));
    }

    #[test]
    fn test_allowed_unused_export_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { foo } from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ReExportedSymbol>> = AHashMap::default();
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_allowed_unused_export_default_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { default as foo } from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);
        assert!(visitor.export_from_ids.is_empty());
    }

    #[test]
    fn test_allowed_unused_export_star_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export * from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);
        module.visit_with(&mut visitor);

        assert!(visitor.export_from_ids.is_empty());
    }

    #[test]
    fn test_export_named() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            const foo = 1;
            export { foo }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Named("foo".to_owned()));

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            // some comment
            const foo = 1;
            export { foo as bar };
            
            // another comment
            // @ALLOW-UNUSED-EXPORT this are some docs
            export const zoo = 2;
            "#
            .to_string(),
        );

        let comments = SingleThreadedComments::default();

        let mut parser = create_test_parser(&fm, Some(&comments));
        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);

        module.visit_with(&mut visitor);
        assert_eq!(visitor.exported_ids.len(), 2);
        assert!(
            visitor.exported_ids.iter().any(|(symbol, e)| *symbol
                == ExportedSymbol::Named("bar".to_owned())
                || !e.allow_unused),
            "`bar` export should not be allowed unused"
        );
        assert!(
            visitor.exported_ids.iter().any(|(symbol, e)| *symbol
                == ExportedSymbol::Named("zoo".to_owned())
                || e.allow_unused),
            "`zoo` export should be allowed unused"
        );
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            // some comment
            const foo = 1;
            export default foo;
            
            // another comment
            // @ALLOW-UNUSED-EXPORT this are some docs
            export const zoo = 2;
            "#
            .to_string(),
        );

        let comments = SingleThreadedComments::default();

        let mut parser = create_test_parser(&fm, Some(&comments));
        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);

        module.visit_with(&mut visitor);
        assert_eq!(visitor.exported_ids.len(), 2);
        assert!(
            visitor.exported_ids.iter().any(|(symbol, e)| *symbol
                == ExportedSymbol::Named("foo".to_owned())
                || !e.allow_unused),
            "`bar` export should not be allowed unused"
        );
        assert!(
            visitor
                .exported_ids
                .iter()
                .any(|(symbol, e)| *symbol == ExportedSymbol::Default || e.allow_unused),
            "`zoo` export should be allowed unused"
        );
    }

    #[test]
    fn test_allow_unused_export_default_and_collect_not_marked_named_export() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            // some comment
            const foo = 1;
            // @ALLOW-UNUSED-EXPORT this are some docs
            export default foo;
            
            // another comment
            export const zoo = 2;
            "#
            .to_string(),
        );

        let comments = SingleThreadedComments::default();

        let mut parser = create_test_parser(&fm, Some(&comments));
        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsVisitor::new(comments);

        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> =
            aset!(ExportedSymbol::Named("zoo".to_string()));

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_named_as_bar() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            const foo = 1;
            export { foo as bar }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Named("bar".to_owned()));

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            const foo = 1;
            export default foo;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Default);

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_kind_as_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            interface Foo {
                bar: boolean;
            }
            export type { Foo as default };
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Default);

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_default_execution() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            function foo() { return 1; }
            export default foo();
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Default);

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_default_class() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            export default class Foo {}
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Default);

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_const() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            export const foo = 1;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashSet<ExportedSymbol> = aset!(ExportedSymbol::Named("foo".to_owned()));

        assert_eq!(
            expected_map,
            visitor
                .exported_ids
                .drain()
                .map(|(k, _)| k)
                .collect::<AHashSet<_>>()
        );
    }

    #[test]
    fn test_export_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            export { foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ReExportedSymbol>> = amap!( "./foo" =>
            aset!(
                ReExportedSymbol{
                    imported: ExportedSymbol::Named("foo".to_owned()),
                    renamed_to: None,
                }
            )
        );
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_default_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            export { default as foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ReExportedSymbol>> = amap!(
        "./foo" => aset!(ReExportedSymbol{
            imported: ExportedSymbol::Default,
            renamed_to: Some(ExportedSymbol::Named("foo".to_owned())),
        }));
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_star_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            export * from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ReExportedSymbol>> = amap!("./foo" => aset!(ReExportedSymbol{
            imported: ExportedSymbol::Namespace,
            renamed_to: None,
        }));
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_import_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import {foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier_with_alias() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import {foo as bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_default_with_alias() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import {default as foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_call() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            const lazyModule = new LazyModule(() => import(/* webpackChunkName: "mailStore" */ './foo'));
            export const lazyModule = new LazyModule(
                () => import(/* webpackChunkName: "SxSStore" */ './lazyIndex')
            );
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);

        assert_eq!(
            aset!("./foo".to_string(), "./lazyIndex".to_string()),
            visitor.imported_paths
        );
    }

    #[test]
    fn test_import_default_and_specifier() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import foo, {bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!(
            "./foo" => aset!(ExportedSymbol::Default, ExportedSymbol::Named("bar".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_star() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import * as foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Namespace));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_require() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            const foo = require('./foo');
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);

        assert_eq!(aset!("./foo".to_owned()), visitor.require_paths);
    }

    #[test]
    fn test_import_equals() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import foo = require('./foo')
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);

        assert_eq!(aset!("./foo".to_owned()), visitor.imported_paths);
    }

    #[test]
    fn test_import_statement() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            Arc::new(FileName::Custom("test.ts".into())),
            r#"
            import './foo'
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsVisitor::new(Default::default());

        let module = parser.parse_typescript_module().unwrap();
        module.visit_with(&mut visitor);

        assert_eq!(aset!("./foo".to_owned()), visitor.executed_paths);
    }
}
