#[cfg(test)]
mod test {

    use ahashmap::{AHashMap, AHashSet};
    use logger::StdioLogger;
    use logger_srcfile::WrapFileLogger;
    use swc_common::comments::SingleThreadedComments;
    use swc_common::sync::Lrc;
    use swc_common::{FileName, SourceMap};

    use crate::parse::{ExportedSymbol, RawImportExportInfo, ReExportedSymbol};

    use test_tmpdir::{amap, amap2, aset};

    /// Parse source via `ast_segmenter::segment_file`, then flatten the
    /// resulting segments into a `RawImportExportInfo`.
    fn parse(src: &str) -> RawImportExportInfo {
        let cm = Lrc::<SourceMap>::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(
            Lrc::new(FileName::Custom("test.ts".into())),
            src.to_string(),
        );

        let lexer = swc_utils_parse::create_lexer(&fm, Some(&comments));
        let capturing = swc_ecma_parser::Capturing::new(lexer);
        let mut parser = swc_ecma_parser::Parser::new_from(capturing);
        let module = parser.parse_typescript_module().unwrap();

        let stdio_logger = StdioLogger::new();
        let logger = WrapFileLogger::new("test.ts", src.to_string(), &stdio_logger);

        let segments = ast_segmenter::segment_file(&logger, &module, &comments);
        RawImportExportInfo::from(segments.as_slice())
    }

    #[derive(PartialEq, Debug)]
    struct TestMeta {
        pub allow_unused: bool,
        pub is_typeonly: bool,
    }

    fn own_exported_ids(info: &RawImportExportInfo) -> AHashMap<ExportedSymbol, TestMeta> {
        info.exported_ids
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
        info: &RawImportExportInfo,
    ) -> AHashMap<String, AHashMap<ReExportedSymbol, TestMeta>> {
        info.export_from_ids
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
        let info = parse(
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
            own_exported_ids(&info)
        );
    }

    #[test]
    fn test_allowed_unused_export_named_as_bar() {
        let info = parse(
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
            own_exported_ids(&info)
        );
    }
    #[test]
    fn test_allowed_unused_export_default() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allowed_unused_export_kind_as_default() {
        let info = parse(
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
            own_exported_ids(&info)
        );
    }

    #[test]
    fn test_allowed_unused_export_default_execution() {
        let info = parse(
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
            own_exported_ids(&info)
        );
    }

    #[test]
    fn test_allowed_unused_export_default_class() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allowed_unused_export_const() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allowed_unused_export_from() {
        let info = parse(
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
            re_exported_ids(&info)
        )
    }

    #[test]
    fn test_allowed_unused_export_default_from() {
        let info = parse(
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
            re_exported_ids(&info)
        )
    }

    #[test]
    fn test_allowed_unused_export_star_from() {
        let info = parse(
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
            re_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_named() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allow_unused_export_and_collect_not_marked_export_default() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_allow_unused_export_default_and_collect_not_marked_named_export() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_named_as_bar() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_default() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_kind_as_default() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_default_execution() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_default_class() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_const() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_const_multi() {
        let info = parse(
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
            own_exported_ids(&info)
        )
    }

    #[test]
    fn test_export_from() {
        let info = parse(
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
        assert_eq!(expected_map, re_exported_ids(&info));
    }

    #[test]
    fn test_export_default_from() {
        let info = parse(
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
        assert_eq!(expected_map, re_exported_ids(&info));
    }

    #[test]
    fn test_export_star_from() {
        let info = parse(
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
        assert_eq!(expected_map, re_exported_ids(&info));
    }

    #[test]
    fn test_import_default() {
        let info = parse(
            r#"
            import foo from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_import_specifier() {
        let info = parse(
            r#"
            import {foo} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_import_specifier_with_alias() {
        let info = parse(
            r#"
            import {foo as bar} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!( "./foo" =>
            aset!(ExportedSymbol::Named("foo".to_owned()))
        );
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_import_default_with_alias() {
        let info = parse(
            r#"
            import {default as foo} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Default));
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_import_call() {
        let info = parse(
            r#"
            const lazyModule = new LazyModule(() => import(/* webpackChunkName: "mailStore" */ './foo'));
            export const lazyModule = new LazyModule(
                () => import(/* webpackChunkName: "SxSStore" */ './lazyIndex')
            );
            "#,
        );

        assert_eq!(
            aset!("./foo".to_string(), "./lazyIndex".to_string()),
            info.imported_paths
        );
    }

    #[test]
    fn test_import_default_and_specifier() {
        let info = parse(
            r#"
            import foo, {bar} from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> = amap!(
            "./foo" => aset!(ExportedSymbol::Default, ExportedSymbol::Named("bar".to_owned()))
        );
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_import_star() {
        let info = parse(
            r#"
            import * as foo from './foo';
            "#,
        );
        let expected_map: AHashMap<String, AHashSet<ExportedSymbol>> =
            amap!("./foo" => aset!(ExportedSymbol::Namespace));
        assert_eq!(expected_map, info.imported_path_ids);
    }

    #[test]
    fn test_require() {
        let info = parse(
            r#"
            const foo = require('./foo');
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), info.require_paths);
    }

    #[test]
    fn test_import_equals() {
        let info = parse(
            r#"
            import foo = require('./foo')
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), info.imported_paths);
    }

    #[test]
    fn test_import_statement() {
        let info = parse(
            r#"
            import './foo'
            "#,
        );

        assert_eq!(aset!("./foo".to_owned()), info.executed_paths);
    }

    #[test]
    fn test_realworld_example() {
        let info = parse(
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
            info
                .exported_ids
                .keys()
                .cloned()
                .collect::<AHashSet<_>>()
        );
    }
}
