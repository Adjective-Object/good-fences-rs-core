#[cfg(test)]
mod test {

    use ahashmap::{AHashMap, AHashSet};
    use logger::StdioLogger;
    use logger_srcfile::{SrcFileLogger, WrapFileLogger};
    use swc_common::comments::{Comments, SingleThreadedComments};
    use swc_common::sync::Lrc;
    use swc_common::{FileName, SourceFile, SourceMap};
    use swc_ecma_parser::lexer::Lexer;
    use swc_ecma_parser::{Capturing, Parser};
    use swc_ecma_visit::VisitWith;

    use crate::parse::{ExportedSymbol, ReExportedSymbol};
    use swc_utils_parse::create_lexer;

    use crate::parse::exports_visitor::ExportsVisitor;
    use test_tmpdir::{amap, amap2, aset};

    fn create_test_parser<'a>(
        fm: &'a Lrc<SourceFile>,
        comments: Option<&'a dyn Comments>,
    ) -> Parser<Capturing<Lexer<'a>>> {
        let lexer = create_lexer(fm, comments);
        let capturing = Capturing::new(lexer);

        Parser::new_from(capturing)
    }

    fn visit(src: &str) -> ExportsVisitor<impl SrcFileLogger> {
        let cm = Lrc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Lrc::new(FileName::Custom("test.ts".into())),
            src.to_string(),
        );

        let mut parser = create_test_parser(&fm, Some(&comments));
        let module = parser.parse_typescript_module().unwrap();

        let stdio_logger = StdioLogger::new();
        let logger = WrapFileLogger::new(cm, stdio_logger);

        let mut visitor = ExportsVisitor::new(logger, comments);
        module.visit_with(&mut visitor);

        visitor
    }

    #[derive(PartialEq, Debug)]
    struct TestMeta {
        pub allow_unused: bool,
        pub is_typeonly: bool,
    }

    fn exported_ids(
        visitor: &ExportsVisitor<impl SrcFileLogger>,
    ) -> AHashMap<ExportedSymbol, TestMeta> {
        visitor
            .exported_ids
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    TestMeta {
                        allow_unused: v.allow_unused,
                        is_typeonly: v.is_type_only,
                    },
                )
            })
            .collect()
    }

    fn re_exported_ids(
        visitor: &ExportsVisitor<impl SrcFileLogger>,
    ) -> AHashMap<String, AHashMap<ReExportedSymbol, TestMeta>> {
        visitor
            .export_from_ids
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter()
                        .map(|(re_exported, meta)| {
                            (
                                re_exported.clone(),
                                TestMeta {
                                    allow_unused: meta.allow_unused,
                                    is_typeonly: meta.is_type_only,
                                },
                            )
                        })
                        .collect::<AHashMap<ReExportedSymbol, TestMeta>>(),
                )
            })
            .collect()
    }

    #[test]
    fn test_allowed_unused_export_named() {
        let visitor = visit(
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export { foo }
                "#,
        );
        assert_eq!(
            amap2!(
                "foo".into() => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        );
    }

    #[test]
    fn test_allowed_unused_export_named_as_bar() {
        let visitor = visit(
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export { foo as bar }
                "#,
        );

        assert_eq!(
            amap2!(
                "bar".into() => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        );
    }
    #[test]
    fn test_allowed_unused_export_default() {
        let visitor = visit(
            r#"
                const foo = 1;
                // @ALLOW-UNUSED-EXPORT
                export default foo;
                "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allowed_unused_export_kind_as_default() {
        let visitor = visit(
            r#"
                interface Foo {
                    bar: boolean;
                }
                // @ALLOW-UNUSED-EXPORT
                export type { Foo as default };
                "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: true,
                    is_typeonly: true
                }
            ),
            exported_ids(&visitor)
        );
    }

    #[test]
    fn test_allowed_unused_export_default_execution() {
        let visitor = visit(
            r#"
                function foo() { return 1; }
                // @ALLOW-UNUSED-EXPORT
                export default foo();
                "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        );
    }

    #[test]
    fn test_allowed_unused_export_default_class() {
        let visitor = visit(
            r#"
                // @ALLOW-UNUSED-EXPORT
                export default class Foo {}
                "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allowed_unused_export_const() {
        let visitor = visit(
            r#"
                // @ALLOW-UNUSED-EXPORT
                export const foo = 1;
                "#,
        );
        assert_eq!(
            amap2!(
                "foo".into() => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allowed_unused_export_from() {
        let visitor = visit(
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { foo } from './foo';
                "#,
        );
        assert_eq!(
            amap!(
                "./foo" => amap2!(
                    ReExportedSymbol {
                        imported: ExportedSymbol::Named("foo".to_owned()),
                        renamed_to: None,
                    } => TestMeta {
                        allow_unused: true,
                        is_typeonly: false
                    }
                )
            ),
            re_exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allowed_unused_export_default_from() {
        let visitor = visit(
            r#"
                // @ALLOW-UNUSED-EXPORT
                export { default as foo } from './foo';
                "#,
        );
        assert_eq!(
            amap!(
                "./foo" => amap2!(
                    ReExportedSymbol {
                        imported: ExportedSymbol::Default,
                        renamed_to: Some(ExportedSymbol::Named("foo".to_owned())),
                    } => TestMeta {
                        allow_unused: true,
                        is_typeonly: false
                    }
                )
            ),
            re_exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allowed_unused_export_star_from() {
        let visitor = visit(
            r#"
                // @ALLOW-UNUSED-EXPORT
                export * from './foo';
                "#,
        );
        assert_eq!(
            amap!(
                "./foo" => amap2!(
                    ReExportedSymbol {
                        imported: ExportedSymbol::Namespace,
                        renamed_to: None,
                    } => TestMeta {
                        allow_unused: true,
                        is_typeonly: false
                    }
                )
            ),
            re_exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_named() {
        let visitor = visit(
            r#"
            const foo = 1;
            export { foo }
            "#,
        );
        assert_eq!(
            amap2!(
                "foo".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export() {
        let visitor = visit(
            r#"
            // some comment
            const foo = 1;
            export { foo as bar };
            
            // another comment
            // @ALLOW-UNUSED-EXPORT this are some docs
            export const zoo = 2;
            "#,
        );
        assert_eq!(
            amap2!(
                "bar".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                },
                "zoo".into() => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export_default() {
        let visitor = visit(
            r#"
            // some comment
            const foo = 1;
            export default foo;
            
            // another comment
            // @ALLOW-UNUSED-EXPORT this are some docs
            export const zoo = 2;
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                },
                "zoo".into() => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_allow_unused_export_default_and_collect_not_marked_named_export() {
        let visitor = visit(
            r#"
            // some comment
            const foo = 1;
            // @ALLOW-UNUSED-EXPORT this are some docs
            export default foo;
            
            // another comment
            export const zoo = 2;
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: true,
                    is_typeonly: false
                },
                "zoo".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_named_as_bar() {
        let visitor = visit(
            r#"
            const foo = 1;
            export { foo as bar }
            "#,
        );
        assert_eq!(
            amap2!(
                "bar".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_default() {
        let visitor = visit(
            r#"
            const foo = 1;
            export default foo;
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_kind_as_default() {
        let visitor = visit(
            r#"
            interface Foo {
                bar: boolean;
            }
            export type { Foo as default };
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: false,
                    is_typeonly: true
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_default_execution() {
        let visitor = visit(
            r#"
            function foo() { return 1; }
            export default foo();
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_default_class() {
        let visitor = visit(
            r#"
            export default class Foo {}
            "#,
        );
        assert_eq!(
            amap2!(
                ExportedSymbol::Default => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_const() {
        let visitor = visit(
            r#"
            export const foo = 1;
            "#,
        );
        assert_eq!(
            amap2!(
                "foo".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_const_multi() {
        let visitor = visit(
            r#"
            export const foo = 1, bar = 2;
            "#,
        );
        assert_eq!(
            amap2!(
                "foo".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                },

                "bar".into() => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            ),
            exported_ids(&visitor)
        )
    }

    #[test]
    fn test_export_from() {
        let visitor = visit(
            r#"
            export { foo } from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashMap<ReExportedSymbol, TestMeta>> = amap!(
            "./foo" => amap2!(
                ReExportedSymbol{
                    imported: ExportedSymbol::Named("foo".to_owned()),
                    renamed_to: None,
                } => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            )
        );
        assert_eq!(expected_map, re_exported_ids(&visitor));
    }

    #[test]
    fn test_export_default_from() {
        let visitor = visit(
            r#"
            export { default as foo } from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashMap<ReExportedSymbol, TestMeta>> = amap!(
            "./foo" => amap2!(
                ReExportedSymbol{
                    imported: ExportedSymbol::Default,
                    renamed_to: Some(ExportedSymbol::Named("foo".to_owned())),
                } => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            )
        );
        assert_eq!(expected_map, re_exported_ids(&visitor));
    }

    #[test]
    fn test_export_star_from() {
        let visitor = visit(
            r#"
            export * from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashMap<ReExportedSymbol, TestMeta>> = amap!(
            "./foo" => amap2!(
                ReExportedSymbol{
                    imported: ExportedSymbol::Namespace,
                    renamed_to: None,
                } => TestMeta {
                    allow_unused: false,
                    is_typeonly: false
                }
            )
        );
        assert_eq!(expected_map, re_exported_ids(&visitor));
    }

    #[test]
    fn test_import_default() {
        let visitor = visit(
            r#"
            import foo from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier() {
        let visitor = visit(
            r#"
            import {foo} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_specifier_with_alias() {
        let visitor = visit(
            r#"
            import {foo as bar} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_default_with_alias() {
        let visitor = visit(
            r#"
            import {default as foo} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_call() {
        let visitor = visit(
            r#"
            const lazyModule = new LazyModule(() => import(/* webpackChunkName: "mailStore" */ './foo'));
            export const lazyModule = new LazyModule(
                () => import(/* webpackChunkName: "SxSStore" */ './lazyIndex')
            );
            "#,
        );

        assert_eq!(
            aset!("./foo".to_string(), "./lazyIndex".to_string()),
            visitor.imported_paths
        );
    }

    #[test]
    fn test_import_default_and_specifier() {
        let visitor = visit(
            r#"
            import foo, {bar} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!(
            "./foo" => aset!(ExportedSymbol::Default, ExportedSymbol::Named("bar".to_owned()))
        );
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_import_star() {
        let visitor = visit(
            r#"
            import * as foo from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Namespace));
        assert_eq!(expected_map, visitor.imported_ids_path_name);
    }

    #[test]
    fn test_require() {
        let visitor = visit(
            r#"
            const foo = require('./foo');
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), visitor.require_paths);
    }

    #[test]
    fn test_import_equals() {
        let visitor = visit(
            r#"
            import foo = require('./foo')
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), visitor.imported_paths);
    }

    #[test]
    fn test_import_statement() {
        let visitor = visit(
            r#"
            import './foo'
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), visitor.executed_paths);
    }

    #[test]
    fn test_realworld_example() {
        let visitor = visit(
            r#"
            export const updateWorkplaceSuggestionForDay = mutatorAction();

            export const { getWorkplaceSuggestionForDay, setWorkplaceSuggestionForDay } = createAccessors();            "#,
        );

        assert_eq!(
            aset!(
                ExportedSymbol::Named("updateWorkplaceSuggestionForDay".to_owned()),
                ExportedSymbol::Named("getWorkplaceSuggestionForDay".to_owned()),
                ExportedSymbol::Named("setWorkplaceSuggestionForDay".to_owned())
            ),
            visitor
                .exported_ids
                .keys()
                .cloned()
                .collect::<AHashSet<_>>()
        );
    }
}
