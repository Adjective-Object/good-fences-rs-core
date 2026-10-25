use super::*;

#[test]
fn runtime_contexts_exclude_declaration_siblings_on_cold_warm_and_noop_builds() {
    for (loader, source) in [
        (
            "loader.mjs",
            // Unsupported concat shapes intentionally retain broad capture.
            r#"export const load = locale => import("../../locales/message_".concat(locale, ".json"));"#,
        ),
        (
            "loader.mjs",
            "export const load = name => import(`./${name}`);",
        ),
        ("loader.cjs", "exports.load = name => require(`./${name}`);"),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!("export {{load}} from './dist/Strings/{loader}';"),
        );
        write(dir.root(), &format!("dist/Strings/{loader}"), source);
        for (file, source) in [
            ("index.js", "export const value = 1;"),
            ("index.ts", "export const typed: number = 2;"),
            ("index.mts", "export const module: number = 3;"),
            ("index.cts", "export = 4;"),
            ("nested/value.mjs", "export default 5;"),
            ("types.d.ts/runtime.mjs", "export default 6;"),
            ("value.d.ts.js", "export default 7;"),
        ] {
            write(dir.root(), &format!("dist/Strings/{file}"), source);
        }
        for extension in ["ts", "mts", "cts"] {
            write(
                dir.root(),
                &format!("dist/Strings/index.d.{extension}"),
                "invalid declaration syntax!",
            );
        }
        let config = options();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        for file in [
            "index.js",
            "index.ts",
            "index.mts",
            "index.cts",
            "nested/value.mjs",
            "types.d.ts/runtime.mjs",
            "value.d.ts.js",
        ] {
            assert!(
                cold.product
                    .graph
                    .file_id(&dir.root().join("dist/Strings").join(file))
                    .is_some(),
                "{loader}: missing {file}"
            );
        }
        assert_eq!(cold.product.graph.file_count(), 9);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.product.artifacts, cold.product.artifacts);
        assert_eq!(warm.warnings, cold.warnings);
        assert_eq!(noop.warnings, cold.warnings);
        assert_eq!(cold.warnings.len(), usize::from(source.contains(".concat")));
    }
}

#[test]
fn runtime_context_declaration_changes_do_not_invalidate_or_fail_final_rechecks() {
    for (loader, source) in [
        ("loader.mjs", "export const load = name => import(name);"),
        (
            "loader.mjs",
            "export const load = name => import(`./${name}`);",
        ),
        ("loader.cjs", "exports.load = name => require(`./${name}`);"),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!("export {{load}} from './parts/{loader}'; import './z.css';"),
        );
        write(dir.root(), &format!("parts/{loader}"), source);
        write(dir.root(), "parts/value.mjs", "export const value = 1;");
        write(dir.root(), "z.css", "body {}");
        let config = options();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let first = session.build(&config).unwrap();
        for extension in ["ts", "mts", "cts"] {
            write(
                dir.root(),
                &format!("parts/value.d.{extension}"),
                "invalid declaration syntax!",
            );
        }
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&first.product, &noop.product));
        assert_eq!(noop.counters.source_parses, 0);
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.product.artifacts, first.product.artifacts);

        let root = dir.root().to_owned();
        let observer = observe_preparation(move |path| {
            if path.file_name().is_some_and(|name| name == "z.css") {
                for extension in ["ts", "mts", "cts"] {
                    write(
                        &root,
                        &format!("parts/nested/added.d.{extension}"),
                        "new invalid declarations!",
                    );
                    write(
                        &root,
                        &format!("parts/value.d.{extension}"),
                        "changed invalid declarations!",
                    );
                }
            }
        });
        let rechecked = session.build(&config).unwrap();
        drop(observer);
        assert!(Rc::ptr_eq(&first.product, &rechecked.product));
        assert_eq!(rechecked.counters.source_parses, 0);
        for extension in ["ts", "mts", "cts"] {
            let added = dir.root().join(format!("parts/nested/added.d.{extension}"));
            assert!(
                added.is_file(),
                "preparation observer must add declarations"
            );
            fs::remove_file(added).unwrap();
            fs::remove_file(dir.root().join(format!("parts/value.d.{extension}"))).unwrap();
        }
        let removed = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&first.product, &removed.product));

        let added = dir.root().join("parts/nested/added.mjs");
        let observer = observe_preparation(move |path| {
            if path.file_name().is_some_and(|name| name == "z.css") {
                fs::write(&added, "export default 2;").unwrap();
            }
        });
        let error = format!("{:#}", session.build(&config).err().unwrap());
        drop(observer);
        assert!(
            error.contains("context candidates changed during build"),
            "{error}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
        let changed = session.build(&config).unwrap();
        assert!(!Rc::ptr_eq(&first.product, &changed.product));
        assert!(changed
            .product
            .graph
            .file_id(&dir.root().join("parts/nested/added.mjs"))
            .is_some());
    }
}

#[test]
fn explicit_runtime_declaration_requests_still_fail_with_original_input_errors() {
    for extension in ["ts", "mts", "cts"] {
        let dir = TmpDir::new();
        let file = format!("types.d.{extension}");
        write(
            dir.root(),
            &file,
            "declare const value: number; export {value};",
        );
        for source in [
            format!("import './{file}';"),
            format!("export const load = () => import('./{file}');"),
            "import './loader.cjs';".to_owned(),
        ] {
            write(dir.root(), "main.mjs", &source);
            write(dir.root(), "loader.cjs", &format!("require('./{file}');"));
            let mut session = BuildSession::new(dir.root()).unwrap();
            for _ in 0..2 {
                let error = format!("{:#}", session.build(&options()).err().unwrap());
                assert!(error.contains(&file), "{error}");
                assert!(
                    error.contains("declaration files are not executable inputs"),
                    "{error}"
                );
            }
        }
        let mut config = options();
        config.entries.get_mut("main").unwrap().path = file.into();
        let error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("declaration files are not executable inputs"),
            "{error}"
        );
    }
}
