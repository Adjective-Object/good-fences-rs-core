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
    config.loaders = crate::loaders::LoaderPolicy::default();
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
fn sass_modules_allow_comment_only_trailing_bare_global_scopes() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export * from './style.module.scss';",
    );
    write(
        dir.root(),
        "style.module.scss",
        r#".container {
  .picker {
    :global {
      /* stylelint-disable-next-line selector-class-pattern */
      .ms-BasePicker-selectedItems { display: flex; flex-wrap: wrap; }
    }
  }
}
"#,
    );
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.loaders = crate::loaders::LoaderPolicy::default();
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let text = css(&built, &dir.root().join("style.module.scss"));
    let rules: Vec<_> = text
        .split('}')
        .filter_map(|rule| rule.split_once('{'))
        .collect();
    assert_eq!(rules.len(), 2, "{text}");
    assert_eq!(
        rules[0].1.trim(),
        "/* stylelint-disable-next-line selector-class-pattern */",
        "{text}"
    );
    let locals: Vec<_> = rules[0].0.split_whitespace().collect();
    assert_eq!(locals.len(), 2, "{text}");
    let descendant: Vec<_> = rules[1].0.split_whitespace().collect();
    assert_eq!(
        descendant,
        [locals[0], locals[1], ".ms-BasePicker-selectedItems"],
        "{text}"
    );
    assert!(
        text.contains("display: flex") && text.contains("flex-wrap: wrap"),
        "{text}"
    );
    assert!(
        !text.contains(":global") && !text.contains(".container") && !text.contains(".picker"),
        "{text}"
    );
    let graph = &built.product.graph;
    let file = graph
        .file_id(&dir.root().join("style.module.scss").canonicalize().unwrap())
        .unwrap();
    for name in ["container", "picker"] {
        graph
            .resolve_export(file, name, source_graph::Namespace::Value)
            .unwrap();
    }
    for name in ["ms-BasePicker-selectedItems", "msBasePickerSelectedItems"] {
        assert!(
            graph
                .resolve_optional_export(file, name, source_graph::Namespace::Value)
                .unwrap()
                .is_none(),
            "exported global class {name}"
        );
    }
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
fn sass_compound_suffixes_keep_import_only_naming_and_use_separate() {
    for extension in ["global.scss", "module.scss", "global.sass", "module.sass"] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "import './style.scss';");
        write(
            dir.root(),
            "style.scss",
            &format!("@import './palette.{extension}'; .item {{ color: $color; }}"),
        );
        let (stem, syntax) = extension.rsplit_once('.').unwrap();
        let value = if syntax == "sass" {
            "$color: green\n"
        } else {
            "$color: green;"
        };
        write(
            dir.root(),
            &format!("_palette.{stem}.import.{syntax}"),
            value,
        );
        let mut session = BuildSession::new(dir.root()).unwrap();
        let config = options();
        let imported = session.build(&config).unwrap();
        assert!(
            css(&imported, &dir.root().join("style.scss")).contains("color: green"),
            "{extension}"
        );
        write(
            dir.root(),
            "style.scss",
            "@import './palette'; .item { color: $color; }",
        );
        let inferred = session.build(&config).unwrap();
        assert!(
            css(&inferred, &dir.root().join("style.scss")).contains("color: green"),
            "{extension}"
        );
        write(
            dir.root(),
            &format!("_palette.{extension}"),
            &value.replace("green", "blue"),
        );
        write(
            dir.root(),
            "style.scss",
            &format!("@use './palette.{extension}' as palette; .item {{ color: palette.$color; }}"),
        );
        let used = session.build(&config).unwrap();
        assert!(
            css(&used, &dir.root().join("style.scss")).contains("color: blue"),
            "{extension}"
        );
    }
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
        error.contains("source changed during build") && error.contains("_colors.scss"),
        "{error}"
    );
}

fn stylesheet_map(result: &BuildResult, source: &Path) -> oxc_sourcemap::SourceMap<'static> {
    let output = &result.product.artifacts.stylesheets[&source.canonicalize().unwrap()];
    let map = &result.product.artifacts.resources[&crate::sourcemap::path(output)];
    let map = oxc_sourcemap::SourceMap::from_json_string(std::str::from_utf8(map).unwrap())
        .unwrap()
        .into_owned();
    assert_eq!(
        map.get_file().unwrap(),
        output.file_name().unwrap().to_str().unwrap()
    );
    let footer =
        crate::assembly::relative_request(output, &crate::sourcemap::path(output), true).unwrap();
    assert!(css(result, source).ends_with(&format!("/*# sourceMappingURL={footer} */\n")));
    map
}

fn stylesheet_origin(
    result: &BuildResult,
    stylesheet: &Path,
    output_needle: &str,
    original: &Path,
    line: u32,
    column: u32,
) {
    let text = css(result, stylesheet);
    let offset = text
        .find(output_needle)
        .unwrap_or_else(|| panic!("{output_needle}: {text}"));
    let prefix = &text[..offset];
    let generated_line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let generated_column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
    let map = stylesheet_map(result, stylesheet);
    let table = map.generate_lookup_table();
    let token = map
        .lookup_source_view_token(&table, generated_line, generated_column)
        .unwrap();
    assert_eq!(
        token.get_source().unwrap(),
        url::Url::from_file_path(original.canonicalize().unwrap())
            .unwrap()
            .as_str(),
        "{output_needle}"
    );
    assert_eq!(
        (token.get_src_line(), token.get_src_col()),
        (line, column),
        "{output_needle}"
    );
    assert_eq!(
        token.get_source_content().unwrap(),
        fs::read_to_string(original).unwrap()
    );
}

#[test]
fn sass_source_maps_compose_scoping_urls_imports_values_and_forwarded_mixins() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './reset.css'; export {button} from './style.module.scss';",
    );
    write(
                dir.root(),
                "style.module.scss",
                "@use './palette';\r\n@import './reset.css';\r\n.button { composes: base from './base.module.css';\r\n  @include palette.sized;\r\n  color: palette.$color;\r\n  background: url('./icon.svg');\r\n  animation: spin 1s;\r\n}\r\n@keyframes spin { to { opacity: 0; } }\r\n",
            );
    write(dir.root(), "_palette.scss", "@forward './colors';");
    write(
        dir.root(),
        "_colors.scss",
        "$color: red;\n@mixin sized {\n  padding: 6px;\n}\n",
    );
    write(dir.root(), "reset.css", "/* 😀é */ body { margin: 0 }\r\n");
    write(dir.root(), "base.module.css", ".base { display: block; }");
    write(dir.root(), "icon.svg", "<svg/>");
    let mut config = options();
    config.output.source_maps = true;
    config.loaders = crate::loaders::LoaderPolicy::default();
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let style = dir.root().join("style.module.scss");
    stylesheet_origin(
        &result,
        &style,
        "padding:",
        &dir.root().join("_colors.scss"),
        2,
        2,
    );
    stylesheet_origin(
        &result,
        &style,
        "red",
        &dir.root().join("_colors.scss"),
        0,
        8,
    );
    stylesheet_origin(&result, &style, "background:", &style, 5, 2);
    stylesheet_origin(&result, &style, "animation:", &style, 6, 2);
    // Both the replaced selector and the path-changing URL retain their Sass origin.
    stylesheet_origin(&result, &style, "._", &style, 2, 0);
    let text = css(&result, &style);
    assert!(!text.contains("composes:"));
    assert_eq!(stylesheet_map(&result, &style).get_names().count(), 0);
    let keyframe = &text[text.find("@keyframes ").unwrap() + "@keyframes ".len()..];
    let keyframe = keyframe.split_whitespace().next().unwrap();
    stylesheet_origin(&result, &style, keyframe, &style, 6, 2);
    stylesheet_origin(
        &result,
        &style,
        &format!("@keyframes {keyframe}"),
        &style,
        8,
        0,
    );
    let url = &text[text.find("url(").unwrap()..];
    let url = &url[..url.find(')').unwrap() + 1];
    stylesheet_origin(&result, &style, url, &style, 5, 2);
    stylesheet_origin(
        &result,
        &dir.root().join("reset.css"),
        "margin:",
        &dir.root().join("reset.css"),
        0,
        17,
    );
    assert_eq!(result.product.artifacts.resources.len(), 7);
    for (_, map) in result
        .product
        .artifacts
        .files
        .iter()
        .filter(|(path, _)| path.extension().is_some_and(|extension| extension == "map"))
    {
        let map = oxc_sourcemap::SourceMap::from_json_string(map).unwrap();
        assert!(map
            .get_sources()
            .all(|source| !source.ends_with(".scss") && !source.ends_with(".css")));
    }
}

#[test]
fn sass_source_maps_toggle_warm_disk_invalidate_mapping_only_edits_and_publish() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {item} from './style.module.sass';",
    );
    write(
        dir.root(),
        "style.module.sass",
        "@use './variables'\n.item\n  color: variables.$color\n",
    );
    write(dir.root(), "_variables.sass", "$color: red\n");
    let style = dir.root().join("style.module.sass");
    let variables = dir.root().join("_variables.sass");
    let mut config = options();
    config.loaders = crate::loaders::LoaderPolicy::default();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let off = session.build(&config).unwrap();
    assert!(!css(&off, &style).contains("sourceMappingURL"));
    assert_eq!(off.product.artifacts.resources.len(), 1);
    config.output.source_maps = true;
    let on = session.build(&config).unwrap();
    assert_eq!(on.counters.source_parses, 0);
    assert_ne!(
        off.product.artifacts.stylesheets,
        on.product.artifacts.stylesheets
    );
    stylesheet_origin(&on, &style, "red", &variables, 0, 8);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&on.product, &noop.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(on.product.artifacts, disk.product.artifacts);
    write(
        dir.root(),
        "_variables.sass",
        "// mapping-only edit\n$color: red\n",
    );
    let changed = session.build(&config).unwrap();
    assert_eq!(changed.counters.source_parses, 0);
    assert_ne!(
        on.product.artifacts.stylesheets,
        changed.product.artifacts.stylesheets
    );
    assert!(changed.invalidated.contains(&style));
    assert!(changed.invalidated.contains(&dir.root().join("main.mjs")));
    stylesheet_origin(&changed, &style, "red", &variables, 1, 8);
    let css_body = |result: &BuildResult| {
        css(result, &style)
            .split("\n/*#")
            .next()
            .unwrap()
            .to_owned()
    };
    assert_eq!(css_body(&on), css_body(&changed));
    let output = dir.root().join(".bundler-published");
    let (published_build, published) = session
        .build_and_publish(&config, &output, &crate::publish::Cancellation::default())
        .unwrap();
    for (path, bytes) in published_build.product.artifacts.file_bytes() {
        assert_eq!(fs::read(published.directory.join(path)).unwrap(), bytes);
    }
    config.output.source_maps = false;
    let off_again = session.build(&config).unwrap();
    assert_eq!(off_again.counters.source_parses, 0);
    assert_eq!(off_again.product.artifacts.resources.len(), 1);
    assert!(!css(&off_again, &style).contains("sourceMappingURL"));
}
