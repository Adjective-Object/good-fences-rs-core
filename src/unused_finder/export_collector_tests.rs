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
    use crate::unused_finder::exports_collector::{ExportedItem, ImportedItem};

    use crate::unused_finder::exports_collector::ExportsCollector;

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
                // @ALLOW-UNUSED-EXPORT
                export { foo }
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_named_as_bar() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export { foo as bar }
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();

        assert_eq!(expected_map, visitor.exported_ids);
    }
    #[test]
    fn test_allowed_unused_export_default() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export default foo;
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();
        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_type_as_default() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
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
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();
        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_default_execution() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                function foo() { return 1; }
                // @ALLOW-UNUSED-EXPORT
                export default foo();
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();
        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_default_class() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export default class Foo {}
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();
        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_const() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export const foo = 1;
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::new();

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allowed_unused_export_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { foo } from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::new();
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_allowed_unused_export_default_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { default as foo } from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::new();
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_allowed_unused_export_star_from() {
        let cm = Arc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
                // @ALLOW-UNUSED-EXPORT
                export * from './foo';
                "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));

        let module = parser.parse_typescript_module().unwrap();
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::new();
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_named() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export { foo }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("foo".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
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
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);

        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("bar".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
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
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);

        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_allow_unused_export_default_and_collect_not_marked_named_export() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
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
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), comments);

        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("zoo".to_string())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_named_as_bar() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export { foo as bar }
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("bar".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = 1;
            export default foo;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_type_as_default() {
        let cm = Arc::<SourceMap>::default();
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

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_default_execution() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            function foo() { return 1; }
            export default foo();
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_default_class() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export default class Foo {}
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> = HashSet::from_iter(vec![ExportedItem::Default]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_const() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export const foo = 1;
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashSet<ExportedItem> =
            HashSet::from_iter(vec![ExportedItem::Named("foo".to_owned())]);

        assert_eq!(expected_map, visitor.exported_ids);
    }

    #[test]
    fn test_export_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export { foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Named("foo".to_owned())]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_default_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export { default as foo } from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_export_star_from() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            export * from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Namespace]),
        )]);
        assert_eq!(expected_map, visitor.export_from_ids);
    }

    #[test]
    fn test_import_default() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

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
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

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
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {foo as bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

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
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import {default as foo} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_call() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const lazyModule = new LazyModule(() => import(/* webpackChunkName: "mailStore" */ './foo'));
            export const lazyModule = new LazyModule(
                () => import(/* webpackChunkName: "SxSStore" */ './lazyIndex')
            );
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_string(), "./lazyIndex".to_string()]),
            visitor.imported_paths
        );
    }

    #[test]
    fn test_import_default_and_specifier() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo, {bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

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
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import * as foo from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

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
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            const foo = require('./foo');
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.require_paths
        );
    }

    #[test]
    fn test_import_equals() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo = require('./foo')
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.imported_paths
        );
    }

    #[test]
    fn test_import_statement() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import './foo'
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(std::sync::Arc::new(vec![]), Default::default());

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);

        assert_eq!(
            HashSet::from_iter(vec!["./foo".to_owned()]),
            visitor.executed_paths
        );
    }

    #[test]
    fn test_ignored_regex_pattern() {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            FileName::Custom("test.ts".into()),
            r#"
            import foo, {Bar} from './foo';
            "#
            .to_string(),
        );

        let mut parser = create_test_parser(&fm, None);
        let mut visitor = ExportsCollector::new(
            std::sync::Arc::new(vec![regex::Regex::new("[A-Z].*").unwrap()]),
            Default::default(),
        );

        let module = parser.parse_typescript_module().unwrap();
        visit_module(&mut visitor, &module);
        let expected_map: HashMap<String, HashSet<ImportedItem>> = HashMap::from([(
            "./foo".to_owned(),
            HashSet::from_iter(vec![ImportedItem::Default]),
        )]);
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }
}
