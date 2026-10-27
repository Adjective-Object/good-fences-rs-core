use super::*;

#[test]
fn defaults_cover_all_families_and_resource_boundaries() {
    let policy = LoaderPolicy::default();
    for (suffixes, expected) in [
        (&["js", "mjs", "cjs"][..], Loader::Js),
        (&["jsx", "mjsx", "cjsx"][..], Loader::Jsx),
        (&["ts", "mts", "cts"][..], Loader::Ts),
        (&["tsx", "mtsx", "ctsx"][..], Loader::Tsx),
        (&["json"][..], Loader::Json),
        (&["jsonc"][..], Loader::Jsonc),
        (&["css", "global.css"][..], Loader::Css),
        (&["scss", "global.scss"][..], Loader::Scss),
        (&["sass", "global.sass"][..], Loader::Sass),
        (&["module.css"][..], Loader::CssModules),
        (&["module.scss"][..], Loader::ScssModules),
        (&["module.sass"][..], Loader::SassModules),
        (&["locstring.json"][..], Loader::Locstring),
        (&["bin", "unknown", "JS"][..], Loader::File),
    ] {
        for suffix in suffixes {
            let name = format!("input.{suffix}");
            assert_eq!(policy.select(Path::new(&name)).unwrap(), expected, "{name}");
        }
    }
    assert_eq!(
        policy.select(Path::new("extensionless")).unwrap(),
        Loader::File
    );
}

#[test]
fn overlays_preserve_narrow_rules_and_null_blocks_fallback() {
    let policy = LoaderPolicy::from_overrides([
        (".scss", Some(Loader::ScssModules)),
        (".json", None),
        (".private.locstring.json", None),
        (".theme", Some(Loader::SassModules)),
    ])
    .unwrap();
    for (name, expected) in [
        ("a.scss", Loader::ScssModules),
        ("a.global.scss", Loader::Scss),
        ("a.module.scss", Loader::ScssModules),
        ("a.locstring.json", Loader::Locstring),
        ("a.theme", Loader::SassModules),
    ] {
        assert_eq!(policy.select(Path::new(name)).unwrap(), expected);
    }
    for name in ["a.json", "a.private.locstring.json"] {
        let error = policy.select(Path::new(name)).unwrap_err();
        assert!(format!("{error:#}").contains("rejects input"), "{error:#}");
    }
    let policy = policy.with_override("*", None).unwrap();
    assert!(policy.select(Path::new("a.unknown")).is_err());
    assert_eq!(policy.select(Path::new("a.ts")).unwrap(), Loader::Ts);
    let policy = policy.with_override(".json", Some(Loader::File)).unwrap();
    assert_eq!(policy.select(Path::new("a.json")).unwrap(), Loader::File);
    assert_eq!(policy.rules().get("*"), Some(&None));
}

#[test]
fn selection_uses_only_the_filename_not_directories_or_substrings() {
    let policy = LoaderPolicy::from_overrides([(".theme", Some(Loader::ScssModules))]).unwrap();
    for (path, expected) in [
        (r"directory.module.scss\file.scss", Loader::Scss),
        (r"directory.theme\file", Loader::File),
        (r"directory.locstring.json\file.json", Loader::Json),
        (r"directory.d.ts\file.ts", Loader::Ts),
        (r"directory.node\file.js", Loader::Js),
        ("file.module.scss.backup", Loader::File),
        ("file.theme", Loader::ScssModules),
    ] {
        // Use host components so this test also works on non-Windows hosts.
        let path: std::path::PathBuf = path.split('\\').collect();
        assert_eq!(
            policy.select(&path).unwrap(),
            expected,
            "{}",
            path.display()
        );
    }
}

#[test]
fn declarations_and_native_addons_cannot_be_remapped() {
    let policy = LoaderPolicy::from_overrides([
        (".d.ts", Some(Loader::File)),
        (".d.mts", Some(Loader::Js)),
        (".d.cts", Some(Loader::Json)),
        (".special.node", Some(Loader::File)),
        (".node", Some(Loader::Ts)),
        ("*", Some(Loader::Js)),
    ])
    .unwrap();
    for name in ["a.d.ts", "a.d.mts", "a.d.cts", "a.node", "a.special.node"] {
        let error = policy.select(Path::new(name)).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("declaration files") || message.contains("native addons"),
            "{message}"
        );
    }
    for path in ["", "."] {
        assert!(policy.select(Path::new(path)).is_err());
    }
}

#[test]
fn map_order_does_not_change_effective_policy_or_serialization() {
    let first = r#"{".json":null,".locstring.json":"file",".scss":"scss-modules"}"#;
    let second = r#"{".scss":"scss-modules",".locstring.json":"file",".json":null}"#;
    let first: LoaderPolicy = serde_json::from_str(first).unwrap();
    let second: LoaderPolicy = serde_json::from_str(second).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(
        first.resolution_extensions(),
        second.resolution_extensions()
    );
    assert_eq!(
        serde_json::from_str::<LoaderPolicy>(&serde_json::to_string(&first).unwrap()).unwrap(),
        first
    );
    assert_eq!(
        first.select(Path::new("a.locstring.json")).unwrap(),
        Loader::File
    );
}

#[test]
fn resolution_order_excludes_disabled_suffixes_and_keeps_compounds() {
    let defaults = LoaderPolicy::default();
    let overrides = defaults.rules().keys().map(|suffix| (suffix, None));
    let policy = LoaderPolicy::from_overrides(overrides)
        .unwrap()
        .with_override(".web.ts", Some(Loader::Ts))
        .unwrap()
        .with_override(".zz", Some(Loader::File))
        .unwrap()
        .with_override(".aa", Some(Loader::Js))
        .unwrap()
        .with_override(".a", Some(Loader::Json))
        .unwrap()
        .with_override("*", Some(Loader::File))
        .unwrap();
    assert_eq!(policy.resolution_extensions(), ["a", "aa", "zz", "web.ts"]);
    assert!(defaults.resolution_extensions().contains(&"locstring.json"));
}

#[test]
fn strict_maps_validate_suffixes_labels_duplicates_and_value_shapes() {
    for suffix in [
        "", "css", ".", "..css", ".a..css", ".a/b", r".a\b", ".a?b", ".a#b", ".a b", ".a\nb",
        "*.css", ".css*", ".a:b",
    ] {
        assert!(
            LoaderPolicy::from_overrides([(suffix, Some(Loader::Css))]).is_err(),
            "{suffix:?}"
        );
        assert!(
            LoaderPolicy::default().with_override(suffix, None).is_err(),
            "{suffix:?}"
        );
        let json = serde_json::json!({suffix: "css"});
        assert!(serde_json::from_value::<LoaderPolicy>(json).is_err());
    }
    for input in [
        "null",
        "[]",
        "true",
        r#"{".css":false}"#,
        r#"{".css":1}"#,
        r#"{".css":{}}"#,
        r#"{".css":{"css":null}}"#,
        r#"{".css":[]}"#,
        r#"{".css":"CSS"}"#,
        r#"{".css":"css-module"}"#,
        r#"{".css":"unknown"}"#,
        r#"{".css":null,".css":"css"}"#,
        r#"{".css":"css",".css":null}"#,
    ] {
        assert!(
            serde_json::from_str::<LoaderPolicy>(input).is_err(),
            "{input}"
        );
    }
    assert!(LoaderPolicy::from_overrides([(".x", None), (".x", None)]).is_err());
    assert_eq!(
        serde_json::from_str::<LoaderPolicy>("{}").unwrap(),
        LoaderPolicy::default()
    );
    LoaderPolicy::from_overrides([(".some-custom_part.css", Some(Loader::Css))]).unwrap();
}

#[test]
fn labels_round_trip_and_determine_syntax_without_filename_inference() {
    use StylesheetScoping::{Global, Modules};
    use StylesheetSyntax::{Css, Sass, Scss};
    for (label, loader, script, stylesheet) in [
        ("js", Loader::Js, Some(ScriptSyntax::Js), None),
        ("jsx", Loader::Jsx, Some(ScriptSyntax::Jsx), None),
        ("ts", Loader::Ts, Some(ScriptSyntax::Ts), None),
        ("tsx", Loader::Tsx, Some(ScriptSyntax::Tsx), None),
        ("json", Loader::Json, None, None),
        ("jsonc", Loader::Jsonc, None, None),
        ("css", Loader::Css, None, Some((Css, Global))),
        (
            "css-modules",
            Loader::CssModules,
            None,
            Some((Css, Modules)),
        ),
        ("scss", Loader::Scss, None, Some((Scss, Global))),
        (
            "scss-modules",
            Loader::ScssModules,
            None,
            Some((Scss, Modules)),
        ),
        ("sass", Loader::Sass, None, Some((Sass, Global))),
        (
            "sass-modules",
            Loader::SassModules,
            None,
            Some((Sass, Modules)),
        ),
        ("locstring", Loader::Locstring, None, None),
        ("file", Loader::File, None, None),
    ] {
        let json = serde_json::to_string(label).unwrap();
        assert_eq!(label.parse::<Loader>().unwrap(), loader);
        assert_eq!(serde_json::from_str::<Loader>(&json).unwrap(), loader);
        assert_eq!(serde_json::to_string(&loader).unwrap(), json);
        assert!(serde_json::from_value::<Loader>(serde_json::json!({label: null})).is_err());
        let selected = LoaderPolicy::from_overrides([(".custom", Some(loader))])
            .unwrap()
            .select(Path::new("input.custom"))
            .unwrap();
        assert_eq!(selected.script_syntax(), script, "{label}");
        assert_eq!(selected.stylesheet(), stylesheet, "{label}");
    }
}
