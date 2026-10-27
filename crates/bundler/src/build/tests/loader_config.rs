use super::*;
use crate::loaders::{Loader, LoaderPolicy};

fn policy(rules: &[(&str, Option<Loader>)]) -> LoaderPolicy {
    LoaderPolicy::from_overrides(rules.iter().copied()).unwrap()
}

#[test]
fn selected_custom_scss_modules_keep_native_require_identity_and_original_maps() {
    let dir = TmpDir::new();
    let original = "@use 'palette';\n@use 'spacing';\n.button { color: palette.$color; padding: spacing.$padding; }";
    write(dir.root(), "style.theme", original);
    write(dir.root(), "_palette.theme", "$color: red;");
    write(dir.root(), "spacing/_index.theme", "$padding: 2px * 3;");
    write(
        dir.root(),
        "main.mjs",
        "import styles, {button} from './style.theme'; const required = require('./style.theme'); \
         console.log(button === styles.button, required.button === button, required.default === styles);",
    );
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.loaders = policy(&[(".theme", Some(Loader::ScssModules))]);
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(observe(&built.product.artifacts), b"true true true\n");
    let path = dir.root().join("style.theme");
    let css_path = &built.product.artifacts.stylesheets[&path];
    assert_eq!(css_path.extension().unwrap(), "css");
    let css = String::from_utf8_lossy(&built.product.artifacts.resources[css_path]);
    assert!(
        css.contains("color: red") && css.contains("padding: 6px"),
        "{css}"
    );
    assert!(!css.contains(".button"), "{css}");
    let map_path = crate::sourcemap::path(css_path);
    let map: serde_json::Value =
        serde_json::from_slice(&built.product.artifacts.resources[&map_path]).unwrap();
    assert!(map["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source.as_str().unwrap().ends_with("/style.theme")));
    assert!(map["sourcesContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|content| content == original));
    assert!(Rc::ptr_eq(
        &built.product,
        &session.build(&config).unwrap().product
    ));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.product.artifacts, built.product.artifacts);

    write(
        dir.root(),
        "main.mjs",
        "import url from './style.theme'; console.log(typeof url);",
    );
    config.loaders = policy(&[(".theme", Some(Loader::Scss))]);
    let global = session.build(&config).unwrap();
    let css_path = &global.product.artifacts.stylesheets[&path];
    assert!(
        String::from_utf8_lossy(&global.product.artifacts.resources[css_path]).contains(".button")
    );
    config.loaders = policy(&[(".theme", Some(Loader::File))]);
    let file = session.build(&config).unwrap();
    assert_eq!(observe(&file.product.artifacts), b"string\n");
    assert!(file.product.artifacts.stylesheets.is_empty());
    assert!(file
        .product
        .artifacts
        .resources
        .values()
        .any(|bytes| bytes == original.as_bytes()));
}

#[test]
fn selected_json_jsonc_script_and_typed_syntax_do_not_redispatch_by_filename() {
    let dir = TmpDir::new();
    write(dir.root(), "code.json", "export const code = 40;");
    write(dir.root(), "data.js", "{/* comment */\"value\":2,}");
    write(
        dir.root(),
        "typed.custom",
        "export const typed: number = 3;",
    );
    write(
        dir.root(),
        "view.customx",
        "export const view: unknown = <div>hello</div>;",
    );
    write(
        dir.root(),
        "node_modules/react/package.json",
        r#"{"type":"module","exports":{"./jsx-runtime":"./runtime.mjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/react/runtime.mjs",
        "export function jsx(type, props) { return {type, props}; }",
    );
    write(
        dir.root(),
        "main.mjs",
        "import {code} from './code.json'; import data from './data.js'; import {typed} from './typed.custom'; \
         import {view} from './view.customx'; console.log(code + data.value + typed, view.props.children);",
    );
    let mut config = options();
    config.loaders = policy(&[
        (".json", Some(Loader::Js)),
        (".js", Some(Loader::Jsonc)),
        (".custom", Some(Loader::Ts)),
        (".customx", Some(Loader::Tsx)),
    ]);
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(observe(&built.product.artifacts), b"45 hello\n");
    let typed = dir.root().join("typed.custom");
    let map = session.sources[&typed]
        .transpiled
        .as_ref()
        .unwrap()
        .source_map
        .as_ref()
        .unwrap();
    assert!(map.root().sources().get(0).url().ends_with("/typed.custom"));
    assert_eq!(
        map.root().sources().get(0).content(),
        Some("export const typed: number = 3;")
    );
    let fresh = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(fresh.product.artifacts, built.product.artifacts);
    assert_eq!(fresh.counters.source_parses, 0);
    config.loaders = config
        .loaders
        .with_override(".js", Some(Loader::Json))
        .unwrap();
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(error.contains("data.js: invalid JSON module"), "{error}");
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &built.product));

    write(dir.root(), "view.tsx", "export const view: number = 1;");
    config.entries.get_mut("main").unwrap().path = "view.tsx".into();
    config.loaders = policy(&[(".tsx", Some(Loader::Js))]);
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(error.contains("view.tsx"), "{error}");
    write(dir.root(), "view.tsx", "export const view = <div/>;");
    assert!(session.build(&config).is_err());
    write(dir.root(), "view.tsx", "export const view = 7;");
    let plain = session.build(&config).unwrap();
    assert_eq!(
        plain.product.artifacts.entries["main"].extension().unwrap(),
        "mjs"
    );
    assert!(session.sources[&dir.root().join("view.tsx")]
        .transpiled
        .is_none());
}

#[test]
fn syntax_only_transitions_clear_compiler_maps_even_with_equal_plain_javascript() {
    let dir = TmpDir::new();
    let original = "export const value = 42;\n";
    write(dir.root(), "main.mjs", original);
    let mut config = options();
    config.loaders = policy(&[(".mjs", Some(Loader::Ts))]);
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let typed = session.build(&config).unwrap();
    let path = dir.root().join("main.mjs");
    assert_eq!(&*session.sources[&path].text.text(&path).unwrap(), original);
    assert!(session.sources[&path].transpiled.is_some());
    config.loaders = LoaderPolicy::default();
    let plain = session.build(&config).unwrap();
    assert_eq!(plain.counters.source_parses, 0);
    assert!(session.sources[&path].transpiled.is_none());
    assert!(!Rc::ptr_eq(&plain.product, &typed.product));
    let fresh = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(plain.product.artifacts, fresh.product.artifacts);
    assert!(Rc::ptr_eq(
        &plain.product,
        &session.build(&config).unwrap().product
    ));
}

#[test]
fn locstring_overrides_stop_localization_and_unneeded_catalog_io() {
    let dir = TmpDir::new();
    let original = r#"{"hello":"World","_hello.comment":"greeting"}"#;
    write(dir.root(), "strings.locstring.json", original);
    write(
        dir.root(),
        "main.mjs",
        "import strings, {hello} from './strings.locstring.json'; \
         console.log(hello === strings.hello, Object.keys(strings).join(','));",
    );
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let localized = session.build(&config).unwrap();
    assert_eq!(observe(&localized.product.artifacts), b"true hello\n");
    config.locale.default = "missing-catalog".into();
    config.loaders = policy(&[(".locstring.json", Some(Loader::Json))]);
    write(
        dir.root(),
        "main.mjs",
        "import strings from './strings.locstring.json'; console.log(strings.hello, strings['_hello.comment']);",
    );
    let json = session.build(&config).unwrap();
    assert_eq!(observe(&json.product.artifacts), b"World greeting\n");
    config.loaders = policy(&[(".locstring.json", Some(Loader::File))]);
    write(
        dir.root(),
        "main.mjs",
        "import url from './strings.locstring.json'; console.log(typeof url);",
    );
    let file = session.build(&config).unwrap();
    assert_eq!(observe(&file.product.artifacts), b"string\n");
    assert!(file
        .product
        .artifacts
        .resources
        .values()
        .any(|bytes| bytes == original.as_bytes()));
    assert!(Rc::ptr_eq(
        &file.product,
        &session.build(&config).unwrap().product
    ));
    config.loaders = LoaderPolicy::default();
    assert!(format!("{:#}", session.build(&config).err().unwrap()).contains("locale directory"));
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &file.product));
}

#[test]
fn explicit_file_on_known_syntax_preserves_non_utf8_bytes_and_require_values() {
    for extension in [".scss", ".js"] {
        let dir = TmpDir::new();
        let filename = format!("asset{extension}");
        let original = [0xff, 0, b'$', b'{'];
        fs::write(dir.root().join(&filename), original).unwrap();
        write(
            dir.root(),
            "main.mjs",
            &format!(
                "import url from './{filename}'; const required = require('./{filename}'); console.log(typeof url, url === required);"
            ),
        );
        let mut config = options();
        config.resolver.target_env = TargetEnv::Browser;
        config.loaders = policy(&[(extension, Some(Loader::File))]);
        let mut session = BuildSession::new(dir.root()).unwrap();
        let built = session.build(&config).unwrap();
        assert_eq!(observe(&built.product.artifacts), b"string true\n");
        assert!(built.product.artifacts.stylesheets.is_empty());
        assert!(built
            .product
            .artifacts
            .resources
            .values()
            .any(|bytes| bytes == &original));
        config.loaders = policy(&[(extension, None)]);
        assert!(session.build(&config).is_err());
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &built.product));

        config.loaders = policy(&[(extension, Some(Loader::File))]);
        config.entries.get_mut("main").unwrap().path = filename.into();
        let entry = session.build(&config).unwrap();
        assert_eq!(
            entry.product.artifacts.entries["main"].extension().unwrap(),
            "mjs"
        );
        assert_eq!(observe(&entry.product.artifacts), b"");
    }
}

#[test]
fn loader_null_blocks_cold_memory_disk_disabled_and_stylesheet_closures() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import value from './value.json'; console.log(value);",
        );
        write(dir.root(), "value.json", "42");
        let mut config = options();
        config.cache = cache;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let good = session.build(&config).unwrap();
        config.loaders = policy(&[(".json", None)]);
        for attempt in 0..2 {
            let error = format!("{:#}", session.build(&config).err().unwrap());
            assert!(
                error.contains("rejects input") && error.contains("value.json"),
                "{attempt}: {error}"
            );
            assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &good.product));
        }
        assert!(BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .is_err());
        config.entries.get_mut("main").unwrap().path = "missing.json".into();
        let error = format!("{:#}", session.build(&config).err().unwrap());
        assert!(error.contains("rejects input"), "{error}");
    }

    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './root.css';");
    write(dir.root(), "root.css", "@import './child.scss';");
    write(dir.root(), "child.scss", ".child { color: red; }");
    let mut config = options();
    for selected in [None, Some(Loader::File), Some(Loader::Js)] {
        config.loaders = policy(&[(".scss", selected)]);
        let error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("child.scss")
                && (error.contains("incompatible") || error.contains("rejects input")),
            "{error}"
        );
    }
    write(dir.root(), "main.mjs", "import './root.scss';");
    write(dir.root(), "root.scss", "@use './child.scss';");
    config.loaders = policy(&[(".scss", Some(Loader::Scss)), (".child.scss", None)]);
    // A concrete longer null must survive Sass import resolution too.
    write(dir.root(), "root.scss", "@use './blocked.child.scss';");
    write(dir.root(), "blocked.child.scss", "$value: red;");
    assert!(format!(
        "{:#}",
        BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap()
    )
    .contains("rejects input"));
    write(dir.root(), "main.mjs", "import './root.module.css';");
    write(
        dir.root(),
        "root.module.css",
        ".root { composes: child from './child.module.css'; }",
    );
    write(dir.root(), "child.module.css", ".child {}");
    config.loaders = policy(&[(".module.css", None)])
        .with_override(".root.module.css", Some(Loader::CssModules))
        .unwrap();
    write(dir.root(), "main.mjs", "import './style.root.module.css';");
    write(
        dir.root(),
        "style.root.module.css",
        ".root { composes: child from './child.module.css'; }",
    );
    let error = format!(
        "{:#}",
        BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap()
    );
    assert!(
        error.contains("rejects input") && error.contains("child.module.css"),
        "{error}"
    );
}

#[test]
fn loader_resolution_order_exact_files_compound_rules_and_map_order_are_effective() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export {value} from './choice';");
    write(dir.root(), "choice.js", "export const value = 'js';");
    write(
        dir.root(),
        "choice.ts",
        "export const value: string = 'ts';",
    );
    write(dir.root(), "choice.a", "export const value = 'a';");
    write(
        dir.root(),
        "choice.web.ts",
        "export const value: string = 'web';",
    );
    let mut config = options();
    config.loaders = policy(&[(".a", Some(Loader::Js)), (".web.ts", Some(Loader::Ts))]);
    let mut session = BuildSession::new(dir.root()).unwrap();
    let check = |built: &BuildResult, expected: &str| {
        assert_eq!(
            observe_named(
                &built.product.artifacts,
                "const {value} = await import('./main.mjs'); console.log(value);"
            ),
            format!("{expected}\n").as_bytes()
        );
    };
    let derived = session.build(&config).unwrap();
    check(&derived, "a");
    for (extensions, expected) in [
        (vec![".ts", ".js"], "ts"),
        (vec![".web.ts", ".ts"], "web"),
        (vec!["js", "ts"], "js"),
    ] {
        config.resolution_extensions = Some(extensions.into_iter().map(str::to_owned).collect());
        let built = session.build(&config).unwrap();
        check(&built, expected);
        assert_eq!(
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap()
                .product
                .artifacts,
            built.product.artifacts
        );
    }
    config.loaders = config.loaders.with_override("*", Some(Loader::Js)).unwrap();
    write(dir.root(), "choice", "export const value = 'exact';");
    config.resolution_extensions = Some(vec![]);
    check(&session.build(&config).unwrap(), "exact");
    config.resolution_extensions = Some(vec!["ts".into(), "js".into()]);
    check(&session.build(&config).unwrap(), "exact");

    let first: LoaderPolicy =
        serde_json::from_str(r#"{".json":null,".locstring.json":"file",".scss":"scss-modules"}"#)
            .unwrap();
    let second: LoaderPolicy =
        serde_json::from_str(r#"{".scss":"scss-modules",".locstring.json":"file",".json":null}"#)
            .unwrap();
    write(
        dir.root(),
        "main.mjs",
        "import url from './strings.locstring.json'; console.log(typeof url);",
    );
    write(dir.root(), "strings.locstring.json", "not even JSON");
    config.loaders = first;
    let first = session.build(&config).unwrap();
    assert_eq!(observe(&first.product.artifacts), b"string\n");
    config.loaders = second;
    assert!(Rc::ptr_eq(
        &first.product,
        &session.build(&config).unwrap().product
    ));
    config.resolver.extensions = vec!["ts".into(), "js".into()];
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("BuildOptions.resolution_extensions"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
}
