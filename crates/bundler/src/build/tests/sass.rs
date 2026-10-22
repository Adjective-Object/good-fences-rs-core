use super::*;

fn css(result: &BuildResult, source: &Path) -> String {
    let output = &result.product.artifacts.stylesheets[&source.canonicalize().unwrap()];
    String::from_utf8(result.product.artifacts.resources[output].clone()).unwrap()
}

#[test]
fn sass_modules_compile_before_scoping_and_keep_asset_and_css_imports() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {button} from './style.module.scss';",
    );
    write(
        dir.root(),
        "style.module.scss",
        r#"@use 'palette';
@import './reset.css';
@mixin sized($size) { padding: $size * 2; }
.button { @include sized(3px); color: palette.$color; background: url('./icon.svg'); &:hover { color: blue; } }
"#,
    );
    write(
        dir.root(),
        "_palette.scss",
        "@forward './absent/../colors';",
    );
    write(dir.root(), "_colors.scss", "$color: red;");
    write(dir.root(), "reset.css", "body { margin: 0 }");
    write(dir.root(), "icon.svg", "<svg/>");
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let text = css(&cold, &dir.root().join("style.module.scss"));
    assert!(text.contains("padding: 6px"), "{text}");
    assert!(text.contains("color: red"), "{text}");
    assert!(text.contains(":hover"), "{text}");
    assert!(!text.contains(".button"), "{text}");
    assert!(text.contains("@import"), "{text}");
    assert!(
        !text.contains("icon.svg") && !text.contains("reset.css"),
        "{text}"
    );
    assert_eq!(cold.product.artifacts.resources.len(), 3);
    assert!(cold
        .product
        .artifacts
        .files
        .values()
        .any(|source| source.contains("button")));
    assert!(cold.product.artifacts.resources.keys().all(|path| {
        path.extension()
            .is_some_and(|ext| ext == "css" || ext == "svg")
    }));
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(cold.product.artifacts, disk.product.artifacts);
}

#[test]
fn sass_importer_honors_packages_conditions_aliases_and_import_only_partials() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.scss';");
    write(
        dir.root(),
        "style.scss",
        "@use '~theme' as theme; @use './alias' as local; @import 'legacy'; .item { color: theme.$color; width: local.$width; height: $height; }",
    );
    write(
        dir.root(),
        "node_modules/theme/package.json",
        r#"{"exports":{".":{"node":"./wrong.scss","sass":"./_index.scss","default":"./index.js"}}}"#,
    );
    write(
        dir.root(),
        "node_modules/theme/_index.scss",
        "@forward './colors';",
    );
    write(
        dir.root(),
        "node_modules/theme/_colors.scss",
        "$color: green;",
    );
    write(
        dir.root(),
        "node_modules/theme/index.js",
        "throw new Error('not sass');",
    );
    write(
        dir.root(),
        "node_modules/theme/wrong.scss",
        "@error 'wrong host';",
    );
    write(dir.root(), "local/_size.scss", "$width: 8px;");
    write(dir.root(), "_alias.scss", "$width: 999px;");
    write(dir.root(), "_legacy.import.scss", "$height: 9px;");
    write(dir.root(), "_legacy.scss", "$height: 100px;");
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.resolver.alias.insert(
        "./alias".into(),
        dir.root().join("local/_size.scss").to_str().unwrap().into(),
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let text = css(&built, &dir.root().join("style.scss"));
    assert!(text.contains("color: green"), "{text}");
    assert!(text.contains("width: 8px"), "{text}");
    assert!(text.contains("height: 9px"), "{text}");
}

#[test]
fn sass_importer_checks_css_before_indexes_and_use_ignores_import_only_files() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.scss';");
    write(
        dir.root(),
        "style.scss",
        "@use './theme'; @use './legacy'; .item { color: legacy.$color; }",
    );
    write(dir.root(), "theme.css", ".theme { color: red; }");
    write(dir.root(), "theme/_index.scss", "@error 'CSS must win';");
    write(
        dir.root(),
        "_legacy.import.scss",
        "@error 'import-only is not a use target';",
    );
    write(dir.root(), "_legacy.scss", "$color: green;");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    let text = css(&result, &dir.root().join("style.scss"));
    assert!(
        text.contains(".theme") && text.contains("color: green"),
        "{text}"
    );
}

#[test]
fn sass_dependency_edits_additions_deletions_and_recovery_invalidate_output() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.scss';");
    write(
        dir.root(),
        "style.scss",
        "@use './palette'; .item { color: palette.$color; }",
    );
    write(dir.root(), "palette/_index.scss", "$color: red;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let config = options();
    let first = session.build(&config).unwrap();
    write(dir.root(), "palette/_index.scss", "$color: blue;");
    let edited = session.build(&config).unwrap();
    assert!(!Rc::ptr_eq(&first.product, &edited.product));
    assert!(edited.invalidated.contains(&dir.root().join("style.scss")));
    assert!(edited.invalidated.contains(&dir.root().join("main.mjs")));
    assert!(css(&edited, &dir.root().join("style.scss")).contains("color: blue"));
    write(dir.root(), "_palette.scss", "$color: green;");
    let added = session.build(&config).unwrap();
    assert!(css(&added, &dir.root().join("style.scss")).contains("color: green"));
    fs::remove_file(dir.root().join("_palette.scss")).unwrap();
    let removed = session.build(&config).unwrap();
    assert!(css(&removed, &dir.root().join("style.scss")).contains("color: blue"));
    fs::remove_file(dir.root().join("palette/_index.scss")).unwrap();
    assert!(session.build(&config).is_err());
    write(dir.root(), "palette/_index.scss", "$color: purple;");
    let recovered = session.build(&config).unwrap();
    assert!(css(&recovered, &dir.root().join("style.scss")).contains("color: purple"));
}

#[test]
fn sass_indented_syntax_imports_partials_and_surfaces_diagnostics() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.sass';");
    write(
        dir.root(),
        "style.sass",
        "@use './variables'\n@warn 'check sass warning'\n.item\n  color: variables.$color\n  &:hover\n    color: blue\n",
    );
    write(dir.root(), "_variables.sass", "$color: red\n");
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    let text = css(&built, &dir.root().join("style.sass"));
    assert!(
        text.contains("color: red") && text.contains(".item:hover"),
        "{text}"
    );
    assert!(built
        .warnings
        .iter()
        .any(|warning| warning.contains("check sass warning")));
    write(dir.root(), "_variables.sass", "$color: (\n");
    let error = format!(
        "{:#}",
        BuildSession::new(dir.root())
            .unwrap()
            .build(&options())
            .err()
            .unwrap()
    );
    assert!(
        error.contains("Sass compilation failed") && error.contains("variables"),
        "{error}"
    );
}

#[test]
fn sass_importer_rejects_ambiguous_missing_and_out_of_inventory_inputs() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.scss';");
    write(dir.root(), "style.scss", "@use './colors';");
    write(dir.root(), "_colors.scss", "$color: red;");
    write(dir.root(), "colors.scss", "$color: blue;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let error = format!("{:#}", session.build(&options()).err().unwrap());
    assert!(error.contains("ambiguous Sass import"), "{error}");
    fs::remove_file(dir.root().join("_colors.scss")).unwrap();
    fs::remove_file(dir.root().join("colors.scss")).unwrap();
    let error = format!("{:#}", session.build(&options()).err().unwrap());
    assert!(error.contains("cannot resolve Sass import"), "{error}");
    write(dir.root(), "target/_secret.scss", "$color: red;");
    write(dir.root(), "style.scss", "@use './target/secret';");
    let error = format!("{:#}", session.build(&options()).err().unwrap());
    assert!(
        error.contains("outside bounded source inventory"),
        "{error}"
    );
}

#[test]
fn sass_rechecks_transitive_original_bytes_before_returning() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './a.scss'; import './z.css';",
    );
    write(
        dir.root(),
        "a.scss",
        "@use './colors'; .item { color: colors.$color; }",
    );
    write(dir.root(), "_colors.scss", "$color: red;");
    write(dir.root(), "z.css", "body { margin: 0 }");
    let colors = dir.root().join("_colors.scss");
    let _observer = observe_preparation(move |path| {
        if path.file_name().is_some_and(|name| name == "z.css") {
            fs::write(&colors, "$color: blue;").unwrap();
        }
    });
    let error = format!(
        "{:#}",
        BuildSession::new(dir.root())
            .unwrap()
            .build(&options())
            .err()
            .unwrap()
    );
    assert!(
        error.contains("CSS dependency changed during build"),
        "{error}"
    );
}
