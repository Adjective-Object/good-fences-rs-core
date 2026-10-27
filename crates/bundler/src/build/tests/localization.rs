use super::*;

fn config(locale: &str) -> BuildOptions {
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.locale.default = locale.into();
    config
}

#[test]
fn localization_react_shaped_tsx_and_dynamic_activation_stay_native() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"overflowLabel":"More","_overflowLabel.comment":"menu"}"#,
    );
    write(
        dir.root(),
        "view.tsx",
        "import {overflowLabel} from './strings.locstring.json';
         console.log('view');
         export const view = <button title={overflowLabel}>{overflowLabel}</button>;",
    );
    write(
        dir.root(),
        "node_modules/react/package.json",
        r#"{"exports":{"./jsx-runtime":"./runtime.mjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/react/runtime.mjs",
        "export function jsx(type, props) { return {type, props}; } export {jsx as jsxs};",
    );
    write(
        dir.root(),
        "main.mjs",
        "console.log('before'); const load = () => import('./view.tsx');
         console.log('not loaded'); const {view} = await load();
         console.log(view.type, view.props.title, view.props.children);",
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config("none"))
        .unwrap();
    assert_eq!(
        observe(&built.product.artifacts),
        b"before\nnot loaded\nview\nbutton More More\n"
    );
    assert!(!built.product.artifacts.async_facades.is_empty());
}

#[test]
fn localization_native_named_default_namespace_require_and_lazy_exports() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{
        "overflowLabel":"More", "_overflowLabel.comment":"menu",
        "reactLabel":"Hello {name}", "_reactLabel.comment":"greeting",
        "__proto__":"own", "not-an-identifier":"safe", "default":"data",
        "_metadata":"invisible", "unused":"TREE_SHAKE_SENTINEL"
    }"#,
    );
    write(dir.root(), "plain.json", r#"{"_metadata":"ordinary"}"#);
    write(
        dir.root(),
        "reader.cjs",
        "module.exports = () => require('./strings.locstring.json');",
    );
    write(
        dir.root(),
        "lazy.mjs",
        "export {reactLabel} from './strings.locstring.json';",
    );
    write(dir.root(), "main.mjs", "
        import strings from './strings.locstring.json' with {type: 'json'};
        import {overflowLabel, 'not-an-identifier' as safe} from './strings.locstring.json';
        import * as ns from './strings.locstring.json';
        import read from './reader.cjs';
        import plain from './plain.json';
        const lazy = await import('./lazy.mjs');
        const required = require('./strings.locstring.json');
        console.log(JSON.stringify([overflowLabel, strings.overflowLabel, ns.overflowLabel,
          safe, strings.__proto__, Object.hasOwn(strings, '__proto__'), strings.default,
          '_metadata' in strings, '_metadata' in ns, read() === strings,
          lazy.reactLabel, plain._metadata, required === strings,
          require('./strings.locstring.json') === required, required.overflowLabel === overflowLabel,
          read() === required]));");
    for target in [TargetEnv::Browser, TargetEnv::Node] {
        let mut config = config("none");
        config.resolver.target_env = target;
        config.output.source_maps = true;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert_eq!(cold.product.artifacts.backend, link::Backend::Native);
        insta::allow_duplicates! {
            insta::assert_snapshot!(String::from_utf8(observe(&cold.product.artifacts)).unwrap(),
                @r#"["More","More","More","safe",{},false,"data",false,false,true,"Hello {name}","ordinary",true,true,true,true]"#);
        }
        assert_eq!(
            cold.warnings
                .iter()
                .filter(|warning| warning.contains("translation comment"))
                .count(),
            3
        );
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        assert_eq!(cold.warnings, noop.warnings);
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(disk.counters.source_parses, 0);
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
        assert_eq!(cold.warnings, disk.warnings);
        assert_eq!(
            observe(&disk.product.artifacts),
            observe(&cold.product.artifacts)
        );
        config.cache = CacheMode::Disabled;
        let uncached = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, uncached.product.artifacts);
    }
    write(
        dir.root(),
        "main.mjs",
        "import {overflowLabel} from './strings.locstring.json'; console.log(overflowLabel);",
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config("none"))
        .unwrap();
    assert_eq!(observe(&built.product.artifacts), b"More\n");
    assert!(built
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("TREE_SHAKE_SENTINEL")));
}

#[test]
fn localization_pseudo_padding_overflow_returns_source_error() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {a} from './strings.locstring.json'; console.log(a);",
    );
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"a":"ab","_a.comment":"a"}"#,
    );
    for extend in [1e308, isize::MAX as f64 / 4.0] {
        let mut config = config("ploc");
        config.locale.ploc.extend = Some(extend);
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap();
        if extend < 1e308 {
            assert!(error
                .downcast_ref::<PreparationErrors>()
                .unwrap()
                .errors()
                .values()
                .flatten()
                .any(|error| error
                    .downcast_ref::<std::collections::TryReserveError>()
                    .is_some()));
        }
        let message = format!("{error:#}");
        assert!(
            message.contains("pseudo-localizing") && message.contains("strings.locstring.json"),
            "{message}"
        );
        assert!(
            message.contains("padding") || message.contains("capacity"),
            "{message}"
        );
    }
}

#[test]
fn localization_node_esm_require_is_limited_to_localized_resources() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"value":"localized","_value.comment":"value"}"#,
    );
    write(
        dir.root(),
        "main.mjs",
        "const strings = require('./strings.locstring.json'); console.log(strings.value);",
    );
    let mut config = config("none");
    config.resolver.target_env = TargetEnv::Node;
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe(&built.product.artifacts), b"localized\n");
    write(
        dir.root(),
        "main.mjs",
        "console.log(require('./plain.mjs'));",
    );
    write(dir.root(), "plain.mjs", "export const value = 'ordinary';");
    let error = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("native ESM synchronous require target is unsupported"));
}

#[test]
fn localization_catalog_edits_match_sets_failures_and_cache_modes() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {a,b,c} from './strings.locstring.json'; console.log(a,b,c);",
    );
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"a":"sourceA","_a.comment":"a","b":"sourceB","_b.comment":"b","c":"sourceC","_c.comment":"c"}"#,
    );
    write(
        dir.root(),
        "locale/fr/first.locstrings.json",
        r#"{"a":"traduit","b":""}"#,
    );
    let mut config = config("fr");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(
        observe(&cold.product.artifacts),
        b"traduit sourceB sourceC\n"
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        config.cache = cache;
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
        if cache != CacheMode::Disabled {
            assert_eq!(disk.counters.source_parses, 0);
        }
    }
    config.cache = CacheMode::ReadWrite;
    write(
        dir.root(),
        "locale/fr/nested/second.locstrings.json",
        r#"{"c":"ajoute"}"#,
    );
    let added = session.build(&config).unwrap();
    assert_eq!(
        observe(&added.product.artifacts),
        b"traduit sourceB ajoute\n"
    );
    assert!(!Rc::ptr_eq(&cold.product, &added.product));
    write(
        dir.root(),
        "locale/fr/first.locstrings.json",
        r#"{"a":"modifie","b":""}"#,
    );
    let disk_edited = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe(&disk_edited.product.artifacts),
        b"modifie sourceB ajoute\n"
    );
    assert_eq!(disk_edited.counters.source_parses, 1);
    let edited = session.build(&config).unwrap();
    assert_eq!(
        observe(&edited.product.artifacts),
        b"modifie sourceB ajoute\n"
    );
    fs::rename(
        dir.root().join("locale/fr/nested/second.locstrings.json"),
        dir.root().join("locale/fr/nested/renamed.locstrings.json"),
    )
    .unwrap();
    let renamed = session.build(&config).unwrap();
    assert!(!Rc::ptr_eq(&edited.product, &renamed.product));
    fs::remove_file(dir.root().join("locale/fr/nested/renamed.locstrings.json")).unwrap();
    let deleted = session.build(&config).unwrap();
    assert_eq!(
        observe(&deleted.product.artifacts),
        b"modifie sourceB sourceC\n"
    );
    write(
        dir.root(),
        "locale/fr/duplicate.locstrings.json",
        r#"{"a":"duplicate"}"#,
    );
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("duplicate localization catalog key")
            && error.contains("first.locstrings.json")
            && error.contains("duplicate.locstrings.json"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &deleted.product));
    fs::remove_file(dir.root().join("locale/fr/duplicate.locstrings.json")).unwrap();
    let recovered = session.build(&config).unwrap();
    assert_eq!(
        observe(&recovered.product.artifacts),
        b"modifie sourceB sourceC\n"
    );
    for invalid in [r#"{"a":42}"#, "{broken", "[]"] {
        write(dir.root(), "locale/fr/first.locstrings.json", invalid);
        let error = session.build(&config).err().unwrap();
        let failures = error.downcast_ref::<PreparationErrors>().unwrap();
        assert!(failures
            .errors()
            .values()
            .flatten()
            .any(|error| error.chain().any(|cause| cause.is::<serde_json::Error>())));
        assert!(format!("{error:#}").contains("first.locstrings.json"));
    }
    config.locale.default = "none".into();
    assert_eq!(
        observe(&session.build(&config).unwrap().product.artifacts),
        b"sourceA sourceB sourceC\n"
    );
    config.locale.default = "missing".into();
    assert!(format!("{:#}", session.build(&config).err().unwrap()).contains("locale directory"));
    fs::create_dir_all(dir.root().join("locale/empty")).unwrap();
    config.locale.default = "empty".into();
    let empty = session.build(&config).unwrap();
    assert!(empty
        .warnings
        .iter()
        .any(|warning| warning.contains("no localization catalogs")));
    write(dir.root(), "locale/empty/empty.locstrings.json", "{}");
    let empty = session.build(&config).unwrap();
    assert!(empty
        .warnings
        .iter()
        .any(|warning| warning.contains("contained no translations")));
}

#[test]
fn localization_pseudo_options_original_identity_and_diagnostics_invalidate() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import strings from './strings.locstring.json'; console.log(JSON.stringify(strings));",
    );
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"a":"Hello {name}","_a.comment":"greeting","b":"Keep"}"#,
    );
    let mut config = config("PsEuDo-LoCsTrInGs");
    config.locale.ploc.prepend = Some("[!!".into());
    config.locale.ploc.append = Some("!!]".into());
    config.locale.ploc.extend = Some(0.2);
    config.locale.ploc.omit_keys.insert("b".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    assert_eq!(
        observe(&first.product.artifacts),
        "{\"a\":\"[!!Ĥĕľŀō {name}  !!]\",\"b\":\"Keep\"}\n".as_bytes()
    );
    config.locale.ploc.override_ = Some("x".into());
    let changed = session.build(&config).unwrap();
    assert_eq!(
        observe(&changed.product.artifacts),
        b"{\"a\":\"[!!xxxxxx{name}  !!]\",\"b\":\"Keep\"}\n"
    );
    // Original bytes, not just generated JS, govern resource reuse.
    write(
        dir.root(),
        "strings.locstring.json",
        r#"{"a":"Hello {name}","_a.comment":"changed!","b":"Keep","_b.comment":"added"}"#,
    );
    let original = session.build(&config).unwrap();
    assert!(!Rc::ptr_eq(&changed.product, &original.product));
    assert!(original.warnings.is_empty());
    assert_eq!(original.counters.source_parses, 1);
    let lease =
        link::SourceTexts::text(&session.sources, &dir.root().join("strings.locstring.json"))
            .unwrap()
            .unwrap();
    write(dir.root(), "strings.locstring.json", r#"{"a":null}"#);
    let error =
        link::SourceTexts::text(&session.sources, &dir.root().join("strings.locstring.json"))
            .err()
            .unwrap();
    assert!(format!("{error:#}").contains("source changed during build"));
    assert!(lease.contains("xxxxxx"));
    let error = session.build(&config).err().unwrap();
    assert!(format!("{error:#}").contains("strings.locstring.json"));
}

#[test]
fn localization_catalog_consumed_bytes_and_candidate_sets_are_rechecked() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "locale/fr/one.locstrings.json",
        r#"{"a":"original"}"#,
    );
    let root = dir.root().canonicalize().unwrap();
    let options = config("fr");
    let catalog =
        super::super::localization::Localization::new(&Inventory::new(&root), &options.locale)
            .unwrap();
    write(
        dir.root(),
        "locale/fr/one.locstrings.json",
        r#"{"a":"modified"}"#,
    );
    assert!(catalog.recheck().is_err());
    let catalog =
        super::super::localization::Localization::new(&Inventory::new(&root), &options.locale)
            .unwrap();
    write(
        dir.root(),
        "locale/fr/two.locstrings.json",
        r#"{"b":"added"}"#,
    );
    assert!(format!("{:#}", catalog.recheck().err().unwrap()).contains("candidates changed"));
}
