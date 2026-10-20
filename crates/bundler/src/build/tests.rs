use super::*;
use test_tmpdir::TmpDir;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn options() -> BuildOptions {
    BuildOptions {
        entries: BTreeMap::from([(
            "main".into(),
            Entry {
                path: "main.mjs".into(),
                output_name: None,
                namespace: true,
            },
        )]),
        ..Default::default()
    }
}

#[test]
fn css_configuration_transitions_keep_javascript_warm_and_low_level_sources_prepared() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './style.module.css'; console.log('ready');",
    );
    write(
        dir.root(),
        "style.module.css",
        "@import './child.scope'; .used, .unused { color: red }",
    );
    write(dir.root(), "child.scope", ".child { color: blue }");
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = parity(&mut session, &config);
    assert_eq!(cold.counters.source_parses, 2);
    let mut previous = cold;
    for (selection, reparses) in [
        (CssModules::Enabled, 1),
        (CssModules::Everything, 0),
        (CssModules::Extension(".scope".into()), 1),
        (CssModules::Extension(".module.css".into()), 1),
        (CssModules::Enabled, 0),
    ] {
        config.css.css_modules = selection;
        let changed = parity(&mut session, &config);
        assert_eq!(changed.counters.source_parses, reparses);
        assert_eq!(changed.counters.memory_hits, 2 - reparses);
        assert_eq!(changed.counters.source_reparses, 0);
        assert_eq!(changed.counters.fragment_reparses, 0);
        assert_eq!(
            previous.product.fingerprints.roots,
            changed.product.fingerprints.roots
        );
        assert_eq!(
            previous.product.fingerprints.resolution,
            changed.product.fingerprints.resolution
        );
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(disk.counters.disk_hits, 2);
        assert_eq!(disk.counters.source_parses, 0);
        assert_eq!(disk.product.artifacts, changed.product.artifacts);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&noop.product, &changed.product));
        assert_eq!(noop.counters.output_parses, 0);
        previous = changed;
    }
    let path = dir.root().join("style.module.css");
    let parse = previous.product.fingerprints.parse;
    let stylesheet = previous.product.artifacts.stylesheets[&path].clone();
    config.output.base_url = "/static/".into();
    let rebased = parity(&mut session, &config);
    assert_eq!(rebased.counters.memory_hits, 2);
    assert_eq!(rebased.product.fingerprints.parse, parse);
    assert_ne!(
        rebased.product.fingerprints.output,
        previous.product.fingerprints.output
    );
    assert_eq!(rebased.product.artifacts.stylesheets[&path], stylesheet);
    assert_ne!(
        rebased.product.artifacts.resources,
        previous.product.artifacts.resources
    );
}

#[test]
fn css_modules_custom_script_deselection_matches_fresh_error_and_preserves_last_product() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './style.style.mjs'; console.log('ready');",
    );
    write(dir.root(), "style.style.mjs", ".used { color: red }");
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".style.mjs".into());
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        config.css.css_modules = CssModules::Extension(".style.mjs".into());
        let mut session = BuildSession::new(dir.root()).unwrap();
        let selected = session.build(&config).unwrap();
        let original = selected.product.artifacts.clone();
        let warm = session.build(&config).unwrap();
        assert_eq!(warm.counters.memory_hits, 2);
        assert_eq!(warm.counters.source_parses, 0);
        assert!(Rc::ptr_eq(&warm.product, &selected.product));
        let script = dir.root().join("main.mjs");
        let record = session.sources[&script].record.clone();

        config.css.css_modules = CssModules::Disabled;
        let fresh_error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .expect("deselected CSS is not valid JavaScript")
        );
        let error = format!(
            "{:#}",
            session
                .build(&config)
                .err()
                .expect("memory reuse must not bypass JavaScript validation")
        );
        assert_eq!(error, fresh_error);
        assert!(error.contains("style.style.mjs"), "{error}");
        assert!(Rc::ptr_eq(
            session.last.as_ref().unwrap(),
            &selected.product
        ));
        assert_eq!(selected.product.artifacts, original);
        assert_eq!(session.sources[&script].record, record);

        config.css.css_modules = CssModules::Extension(".style.mjs".into());
        let recovered = session.build(&config).unwrap();
        assert_eq!(recovered.counters.memory_hits, 2);
        assert_eq!(recovered.counters.source_parses, 0);
        assert!(Rc::ptr_eq(&recovered.product, &selected.product));
    }
}

#[test]
fn css_loader_transitions_preserve_resources_mappings_and_sealed_generated_sources() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export { used } from './style.module.css';",
    );
    write(
        dir.root(),
        "style.module.css",
        ".used, .unused { color: red }",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let caller = session.build(&config).unwrap();
    let path = dir.root().join("style.module.css");
    let caller_text = session.sources[&path].text.clone();
    for load in [CssLoad::AutoMount, CssLoad::CallerLoaded] {
        config.css.load = load;
        let built = session.build(&config).unwrap();
        assert_eq!(built.counters.memory_hits, 1);
        assert_eq!(built.counters.source_parses, 1);
        assert_eq!(
            built.product.artifacts.resources,
            caller.product.artifacts.resources
        );
        assert_eq!(
            built.product.artifacts.stylesheets,
            caller.product.artifacts.stylesheets
        );
        assert!(built.product.artifacts.async_modules.is_empty());
        let graph = &built.product.graph;
        let file = graph.file_id(&path).unwrap();
        let before = ast_segmenter::module_parse_count();
        let parsed =
            crate::emit::checked_source(graph, file, &session.sources[&path].text).unwrap();
        assert!(!parsed.provenance().config().commonjs);
        assert!(parsed.facts.top_level_await.is_empty());
        assert!(crate::emit::checked_source(graph, file, ".used, .unused { color: red }").is_err());
        if load == CssLoad::AutoMount {
            assert!(crate::emit::checked_source(graph, file, &caller_text).is_err());
        }
        assert_eq!(ast_segmenter::module_parse_count(), before);
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(disk.counters.disk_hits, 2);
        assert_eq!(disk.product.artifacts, built.product.artifacts);
        assert_eq!(
            observe_named(&built.product.artifacts, &format!(
                "{CSS_DOM_RECORDER}\nconst ns = await import('./main.mjs'); assert.match(ns.used, /^_[A-Za-z0-9_-]{{10}}$/); assert.equal(links.length, {}); console.log('loader');",
                usize::from(load == CssLoad::AutoMount),
            )),
            b"loader\n",
        );
    }
    assert_eq!(session.sources[&path].text, caller_text);
}

#[test]
fn css_modules_validate_before_graph_loading_and_preserve_failed_last_product() {
    let dir = TmpDir::new();
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".bad..suffix".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(error.contains("invalid css_modules extension"), "{error}");
    assert!(session.sources.is_empty());

    write(dir.root(), "main.mjs", "import './style.module.css';");
    write(
        dir.root(),
        "style.module.css",
        ":global .你好 { color: red }",
    );
    config.css.css_modules = CssModules::Disabled;
    let global = session.build(&config).unwrap();
    assert_eq!(global.product.artifacts.stylesheets.len(), 1);
    config.css.css_modules = CssModules::Enabled;
    write(
        dir.root(),
        "style.module.css",
        ":local(.你好,) { color: red }",
    );
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let error = format!("{:#}", session.build(&config).err().unwrap());
        assert!(
            error.contains("style.module.css: CSS Modules syntax"),
            "{error}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &global.product));
    }
    config.css.css_modules = CssModules::Disabled;
    config.css.load = CssLoad::CallerLoaded;
    write(
        dir.root(),
        "style.module.css",
        ":global .你好 { color: red }",
    );
    assert_eq!(
        session.build(&config).unwrap().product.artifacts,
        global.product.artifacts
    );
}

#[test]
fn css_modules_custom_suffix_reaches_entries_imports_contexts_and_css_closures() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "parts/style.any.custom.extension",
        ".class { color: red }",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".any.custom.extension".into());
    for (entry, main) in [
        ("parts/style.any.custom.extension", ""),
        ("main.mjs", "import './parts/style.any.custom.extension';"),
        (
            "main.mjs",
            "export const load = () => import('./parts/style.any.custom.extension');",
        ),
        (
            "main.cjs",
            "module.exports = require('./parts/style.any.custom.extension');",
        ),
        (
            "main.mjs",
            "export const load = name => import(`./parts/${name}.any.custom.extension`);",
        ),
        ("main.mjs", "import './global.css';"),
    ] {
        if !main.is_empty() {
            write(dir.root(), entry, main);
        }
        write(
            dir.root(),
            "global.css",
            "@import './parts/style.any.custom.extension';",
        );
        config.entries.get_mut("main").unwrap().path = entry.into();
        for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
            config.css.load = load;
            let built = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert!(
                built.product.artifacts.resources.values().any(|bytes| {
                    let text = std::str::from_utf8(bytes).unwrap();
                    text.contains("color: red") && text.contains("._") && !text.contains(".class")
                }),
                "{main}"
            );
            if main.is_empty() {
                let output = built.product.artifacts.entries["main"].to_str().unwrap();
                assert_eq!(
                    observe_named(&built.product.artifacts, &format!(
                        "{CSS_DOM_RECORDER}\nconst ns = await import('./{output}'); assert.deepEqual(Object.keys(ns), ['class']); assert.equal(links.length, {}); console.log('custom');",
                        usize::from(load == CssLoad::AutoMount),
                    )),
                    b"custom\n",
                );
            }
        }
    }
}

#[test]
fn css_modules_animations_are_file_local_without_extra_exports_under_both_loaders() {
    let dir = TmpDir::new();
    let source = r#".spin { animation: 1s ease spin; animation-name: :global(spin), spin; --name: spin }
@media (width > 1px) { @keyframes spin { from { opacity: 0 } to { opacity: 1 } } }
@keyframes :global(remote) {}
"#;
    for file in ["a.module.css", "b.module.css"] {
        write(dir.root(), file, source);
    }
    let global = ".global { animation: spin 1s } @keyframes spin {}";
    write(dir.root(), "global.css", global);
    write(
        dir.root(),
        "main.mjs",
        "export * as a from './a.module.css'; export * as b from './b.module.css'; import './global.css';",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let built = session.build(&config).unwrap();
        let mut animations = Vec::new();
        let mut classes = Vec::new();
        for file in ["a.module.css", "b.module.css"] {
            let resource = &built.product.artifacts.stylesheets[&dir.root().join(file)];
            let css = std::str::from_utf8(&built.product.artifacts.resources[resource]).unwrap();
            let animation = css
                .split("@keyframes ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            let class = css
                .split_whitespace()
                .next()
                .unwrap()
                .strip_prefix('.')
                .unwrap();
            assert_ne!(animation, class);
            assert!(css.contains(&format!("animation: 1s ease {animation}")));
            assert!(css.contains(&format!("animation-name: spin, {animation}")));
            assert!(css.contains("--name: spin") && css.contains("@keyframes remote {}"));
            animations.push(animation.to_owned());
            classes.push(class.to_owned());
        }
        assert_ne!(animations[0], animations[1]);
        let global_resource = &built.product.artifacts.stylesheets[&dir.root().join("global.css")];
        assert_eq!(
            built.product.artifacts.resources[global_resource],
            global.as_bytes()
        );
        assert_eq!(
            observe_named(&built.product.artifacts, &format!(
                "{CSS_DOM_RECORDER}\nconst ns = await import('./main.mjs'); assert.deepEqual(Object.keys(ns.a), ['spin']); assert.deepEqual(Object.keys(ns.b), ['spin']); assert.equal(ns.a.spin, {:?}); assert.equal(ns.b.spin, {:?}); assert.equal(links.length, {}); console.log('animations');",
                classes[0], classes[1], if load == CssLoad::AutoMount { 3 } else { 0 },
            )),
            b"animations\n",
        );
        assert!(Rc::ptr_eq(
            &built.product,
            &session.build(&config).unwrap().product
        ));
    }
}

#[test]
fn css_modules_scopes_fragments_and_urls_share_original_utf8_edits_under_both_loaders() {
    let dir = TmpDir::new();
    let source = r##"@charset "UTF-8";
/* :local(.leave) url(#你好) */
:global(.theme) :local(.button-label), #你好 {
  filter: url(#你好); clip-path: url("#\4f60 好"); mask: url(#button-label);
  --spaced: url(" #你好 "); --escaped: url(" \9 #你\d 好\a ");
  --spaced-unknown: url(" #unknown "); --spaced-global: url(" #remote ");
  --unknown: url(#unknown); --external: url(https://example.test/a.svg#你好);
  --relative: url("./icon.svg#你好"); content: "url(#你好) :local(.leave)";
  --ident: 你好; --color: #abc;
}
:global(#remote) [id="你好"][class="button-label"] { filter: url(#remote) }
"##;
    write(dir.root(), "style.module.css", source);
    write(dir.root(), "icon.svg", "<svg/>");
    write(
        dir.root(),
        "main.mjs",
        "export * from './style.module.css';",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    let mut first = None;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        let observation = observe_named(&cold.product.artifacts, &format!(
            "{CSS_DOM_RECORDER}\nconst ns = await import('./main.mjs'); assert.deepEqual(Object.keys(ns), ['buttonLabel', '你好']); assert.equal(links.length, {}); console.log(JSON.stringify(ns));",
            usize::from(load == CssLoad::AutoMount)
        ));
        let mappings: serde_json::Value = serde_json::from_slice(&observation).unwrap();
        let button = mappings["buttonLabel"].as_str().unwrap();
        let id = mappings["你好"].as_str().unwrap();
        let resource = &cold.product.artifacts.stylesheets[&dir.root().join("style.module.css")];
        let bytes = &cold.product.artifacts.resources[resource];
        let text = std::str::from_utf8(bytes).unwrap();
        assert!(text.contains(&format!(".theme .{button}, #{id}")), "{text}");
        assert_eq!(
            text.matches(&format!("url(\"#{id}\")")).count(),
            4,
            "{text}"
        );
        for untouched in [
            "/* :local(.leave) url(#你好) */",
            "mask: url(#button-label)",
            "url(#unknown)",
            r##"url(" #unknown ")"##,
            r##"url(" #remote ")"##,
            "url(https://example.test/a.svg#你好)",
            r#"content: "url(#你好) :local(.leave)""#,
            "--ident: 你好; --color: #abc",
            r##"#remote [id="你好"][class="button-label"] { filter: url(#remote) }"##,
        ] {
            assert!(text.contains(untouched), "missing {untouched:?}: {text}");
        }
        let asset = cold
            .product
            .artifacts
            .resources
            .keys()
            .find(|path| path.extension().is_some_and(|ext| ext == "svg"))
            .unwrap();
        assert!(
            text.contains(&format!(
                "url(\"./{}#你好\")",
                asset.file_name().unwrap().to_str().unwrap()
            )),
            "{text}"
        );
        if let Some(first) = &first {
            assert_eq!(bytes, first);
        } else {
            first = Some(bytes.clone());
        }
        let memory = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &memory.product));
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
    }
    config.css.css_modules = CssModules::Disabled;
    config.css.load = CssLoad::CallerLoaded;
    let global = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let resource = &global.product.artifacts.stylesheets[&dir.root().join("style.module.css")];
    let text = std::str::from_utf8(&global.product.artifacts.resources[resource]).unwrap();
    assert!(text.contains(":global(.theme) :local(.button-label), #你好"));
    assert!(text.contains(r##"filter: url(#你好); clip-path: url("#\4f60 好")"##));
}

#[test]
fn css_modules_transitive_scope_selection_is_per_stylesheet_not_inherited() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './root.scope';");
    write(
        dir.root(),
        "root.scope",
        "@import './global.css' layer(base) screen; @import './child.scope'; :local(.root) {}",
    );
    write(
        dir.root(),
        "global.css",
        ":global(.theme) .global { filter: url(#target) } #target {}",
    );
    write(
        dir.root(),
        "child.scope",
        "@import './leaf.css'; :global(.theme) :local(.child) {}",
    );
    write(dir.root(), "leaf.css", ".leaf {}");
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".scope".into());
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let styles: Vec<_> = built
        .product
        .artifacts
        .resources
        .values()
        .map(|bytes| std::str::from_utf8(bytes).unwrap())
        .collect();
    assert!(styles.contains(&":global(.theme) .global { filter: url(#target) } #target {}"));
    assert!(styles.contains(&".leaf {}"));
    assert!(styles
        .iter()
        .any(|text| text.contains(".theme ._") && !text.contains(":local")));
    assert!(styles
        .iter()
        .any(|text| text.contains("layer(base) screen;")
            && text.contains("._")
            && !text.contains(":local")));
}

#[test]
fn css_modules_script_and_json_suffixes_preserve_prepared_mode_and_effects() {
    let dir = TmpDir::new();
    let mut config = options();
    config.declarations.user.push(ecco::parse_declarations(
        "global fn document.createElement(tag) {call_effects = false returns.*_effects = false}\n\
         global fn document.head.appendChild(link) {call_effects = false returns.*_effects = false}"
    ).unwrap());
    for suffix in [".module.cjs", ".module.ts", ".module.json"] {
        let stylesheet = format!("style{suffix}");
        write(dir.root(), &stylesheet, ".used, .unused { color: red }");
        config.css.css_modules = CssModules::Extension(suffix.into());
        for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
            config.css.load = load;
            write(
                dir.root(),
                "main.mjs",
                &format!("export {{used}} from './{stylesheet}';"),
            );
            let mut session = BuildSession::new(dir.root()).unwrap();
            let cold = if load == CssLoad::CallerLoaded {
                parity(&mut session, &config)
            } else {
                session.build(&config).unwrap()
            };
            let memory = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&cold.product, &memory.product));
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(cold.product.artifacts, disk.product.artifacts);
            assert_eq!(disk.counters.source_parses, 0);
            assert_eq!(cold.counters.source_reparses, 0);
            assert_eq!(cold.counters.fragment_reparses, 0);
            assert_eq!(
                observe_named(&cold.product.artifacts, &format!(
                    "{CSS_DOM_RECORDER}\nconst ns = await import('./main.mjs'); assert.deepEqual(Object.keys(ns), ['used']); assert.match(ns.used, /^_[A-Za-z0-9_-]{{10}}$/); assert.equal(links.length, {}); console.log('suffix');",
                    usize::from(load == CssLoad::AutoMount),
                )),
                b"suffix\n",
            );
            write(dir.root(), "main.mjs", &format!("import './{stylesheet}';"));
            let unused = session.build(&config).unwrap();
            assert_eq!(unused.product.artifacts.stylesheets.len(), 1);
            if load == CssLoad::AutoMount {
                assert_eq!(
                    observe_named(&unused.product.artifacts, "import assert from 'node:assert/strict'; await assert.rejects(import('./main.mjs'), /CSS auto-mount requires document/); console.log('no document');"),
                    b"no document\n",
                );
            }
            assert_eq!(
                observe_named(&unused.product.artifacts, &format!(
                    "{CSS_DOM_RECORDER}\nawait import('./main.mjs'); assert.equal(links.length, {}); console.log('effects');",
                    usize::from(load == CssLoad::AutoMount),
                )),
                b"effects\n",
            );
        }
    }
}

#[test]
fn caller_loaded_css_module_resource_edits_invalidate_output_without_reparsing_mappings() {
    let dir = TmpDir::new();
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    config.entries.get_mut("main").unwrap().path = "style.module.css".into();
    for with_dependencies in [false, true] {
        let dependency = if with_dependencies {
            "@import './child.css';"
        } else {
            ""
        };
        write(
            dir.root(),
            "style.module.css",
            &format!("{dependency}.used {{color:red}}"),
        );
        write(
            dir.root(),
            "child.css",
            ".child {background:url('./image.bin')}",
        );
        write(dir.root(), "image.bin", "original");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let mut previous = session.build(&config).unwrap();
        let path = dir.root().join("style.module.css");
        let text = session.sources[&path].text.clone();
        let mut edits = vec![(
            "style.module.css",
            format!("{dependency}.used {{color:blue}}"),
        )];
        if with_dependencies {
            edits.extend([
                (
                    "child.css",
                    ".child {background:url('./image.bin');color:green}".into(),
                ),
                ("image.bin", "edited".into()),
            ]);
        }
        for (target, content) in edits {
            write(dir.root(), target, &content);
            let edited = session.build(&config).unwrap();
            assert!(!Rc::ptr_eq(&previous.product, &edited.product), "{target}");
            assert_ne!(
                previous.product.fingerprints.output,
                edited.product.fingerprints.output
            );
            assert_eq!(
                previous.product.fingerprints.parse,
                edited.product.fingerprints.parse
            );
            assert_eq!(session.sources[&path].text, text);
            assert_eq!(edited.counters.source_parses, 0);
            assert_eq!(edited.counters.source_reparses, 0);
            assert_eq!(edited.counters.fragment_reparses, 0);
            assert_ne!(
                previous.product.artifacts.stylesheets[&path],
                edited.product.artifacts.stylesheets[&path],
            );
            let fresh = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(edited.product.artifacts, fresh.product.artifacts);
            let no_op = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&edited.product, &no_op.product));
            previous = edited;
        }
    }
}

#[test]
fn css_modules_everything_selects_css_not_javascript_json_or_assets() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import data from './data.json'; import url from './image.bin'; export { data, url };",
    );
    write(dir.root(), "data.json", "{}");
    write(dir.root(), "image.bin", "binary");
    let mut config = options();
    config.css.css_modules = CssModules::Everything;
    let mut session = BuildSession::new(dir.root()).unwrap();
    assert_eq!(
        session
            .build(&config)
            .unwrap()
            .product
            .artifacts
            .resources
            .len(),
        1
    );
    write(dir.root(), "main.mjs", "import './global.css';");
    write(dir.root(), "global.css", ".global {}");
    let built = session.build(&config).unwrap();
    let path = &built.product.artifacts.stylesheets[&dir.root().join("global.css")];
    let text = std::str::from_utf8(&built.product.artifacts.resources[path]).unwrap();
    assert!(text.starts_with("._"), "{text}");
}

#[test]
fn css_modules_native_named_namespace_dynamic_and_require_surfaces_under_both_loaders() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "style.module.css",
        r".my-class-name, #my-class-name, .class, .\31 23, .a\+b, .你好 { color: red }",
    );
    write(
        dir.root(),
        "main.mjs",
        r#"
import { myClassName, class as keyword, "123" as numeric, "a+b" as punctuation, 你好 as unicode } from './style.module.css';
import * as styles from './style.module.css';
import load from './load.cjs';
export {myClassName, keyword, numeric, punctuation, unicode, styles, load};
export const dynamic = () => import('./style.module.css');
"#,
    );
    write(
        dir.root(),
        "load.cjs",
        "module.exports = () => require('./style.module.css');",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        let memory = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &memory.product));
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
        assert_eq!(disk.counters.source_parses, 0);
        let prepared = &session.sources[&dir.root().join("style.module.css")];
        assert!(!prepared.record.config.commonjs);
        let stylesheet = &cold.product.artifacts.stylesheets[&dir.root().join("style.module.css")];
        let css = std::str::from_utf8(&cold.product.artifacts.resources[stylesheet]).unwrap();
        let check = format!(
            r#"{CSS_DOM_RECORDER}
const main = await import('./main.mjs');
const ns = main.styles;
assert.deepEqual(Object.keys(ns).sort(), ['123', 'a+b', 'class', 'myClassName', '你好'].sort());
assert.equal(ns.myClassName, main.myClassName);
assert.equal(ns.class, main.keyword);
assert.equal(ns['123'], main.numeric);
assert.equal(ns['a+b'], main.punctuation);
assert.equal(ns.你好, main.unicode);
assert.strictEqual(await main.dynamic(), ns);
const required = main.load();
assert.deepEqual(Object.keys(required).sort(), Object.keys(ns).sort());
for (const key of Object.keys(ns)) {{
  assert.equal(required[key], ns[key]);
  assert.match(ns[key], /^_[A-Za-z0-9_-]{{10}}$/);
  assert.ok({css:?}.includes(ns[key]));
}}
assert.ok({css:?}.startsWith('.' + ns.myClassName + ', #' + ns.myClassName));
assert.equal(links.length, {mounts});
console.log('module surfaces');
"#,
            mounts = usize::from(load == CssLoad::AutoMount)
        );
        assert_eq!(
            observe_named(&cold.product.artifacts, &check),
            b"module surfaces\n"
        );
    }
}

#[test]
fn css_modules_prune_bindings_not_stylesheets_and_keep_loader_effects() {
    let dir = TmpDir::new();
    let source = r".used, .unused, .class, .\31 23 { color: red }";
    write(dir.root(), "style.module.css", source);
    write(
        dir.root(),
        "main.mjs",
        "export { used } from './style.module.css';",
    );
    let mappings = crate::css_names::Registry::default()
        .prepare(dir.root(), &dir.root().join("style.module.css"), source)
        .unwrap()
        .mappings;
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    config.declarations.user.push(ecco::parse_declarations(
        "global fn document.createElement(tag) {call_effects = false returns.*_effects = false}\n\
         global fn document.head.appendChild(link) {call_effects = false returns.*_effects = false}"
    ).unwrap());
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let built = session.build(&config).unwrap();
        assert!(built
            .product
            .artifacts
            .files
            .values()
            .any(|text| text.contains(&mappings["used"])));
        for name in ["unused", "class", "123"] {
            assert!(
                built
                    .product
                    .artifacts
                    .files
                    .values()
                    .all(|text| !text.contains(&mappings[name])),
                "{load:?}: {:#?}",
                built.product.artifacts.files
            );
        }
        let path = &built.product.artifacts.stylesheets[&dir.root().join("style.module.css")];
        let css = std::str::from_utf8(&built.product.artifacts.resources[path]).unwrap();
        assert!(css.contains(&mappings["unused"]));
        write(
            dir.root(),
            "main.mjs",
            "import './style.module.css'; export const value = 1;",
        );
        let side_effect_only = session.build(&config).unwrap();
        assert_eq!(side_effect_only.product.artifacts.stylesheets.len(), 1);
        assert!(side_effect_only
            .product
            .artifacts
            .files
            .values()
            .all(|text| !text.contains(&mappings["used"]) && !text.contains(&mappings["unused"])));
        assert_eq!(
            observe_named(&side_effect_only.product.artifacts, &format!(
                "{CSS_DOM_RECORDER}\nawait import('./main.mjs'); assert.equal(links.length, {}); console.log('retained');",
                usize::from(load == CssLoad::AutoMount)
            )),
            b"retained\n"
        );
        write(
            dir.root(),
            "main.mjs",
            "export { used } from './style.module.css';",
        );
    }
}

#[test]
fn css_modules_composition_resolves_before_pruning_under_both_loaders() {
    let dir = TmpDir::new();
    let source = r#"
:local(.button) { composes: mid base mid; color: red; composes: shared shared from global; }
.unused { composes: button; color: blue }
@media screen { .mid { composes: b\61 se; composes: shared from global; } }
.base { color: green }
.button { composes: base; }
"#;
    write(dir.root(), "style.module.css", source);
    write(
        dir.root(),
        "main.mjs",
        "export { button, mid, base } from './style.module.css'; import './global.css';",
    );
    let global = ".global { composes: unknown from './missing.css'; color: black }";
    write(dir.root(), "global.css", global);
    let unused = crate::css_names::Registry::default()
        .prepare(
            dir.root(),
            &dir.root().join("style.module.css"),
            ".unused {}",
        )
        .unwrap()
        .mappings["unused"]
        .clone();
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(
            &cold.product,
            &session.build(&config).unwrap().product
        ));
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
        assert_eq!(disk.counters.source_parses, 0);
        let artifacts = &cold.product.artifacts;
        assert!(artifacts.files.values().all(|text| !text.contains(&unused)));
        let stylesheet = &artifacts.stylesheets[&dir.root().join("style.module.css")];
        let css = std::str::from_utf8(&artifacts.resources[stylesheet]).unwrap();
        assert!(!css.contains("composes"));
        assert!(css.contains(&format!(".{unused}")));
        assert!(css.contains("color: blue"));
        assert!(css.find("color: red").unwrap() < css.find("color: blue").unwrap());
        assert!(css.find("color: blue").unwrap() < css.find("color: green").unwrap());
        let global_path = &artifacts.stylesheets[&dir.root().join("global.css")];
        assert_eq!(artifacts.resources[global_path], global.as_bytes());
        let check = format!(
            r#"{CSS_DOM_RECORDER}
const {{button, mid, base}} = await import('./main.mjs');
const buttonTokens = button.split(' ');
const midTokens = mid.split(' ');
assert.equal(buttonTokens.length, 4);
assert.deepEqual(buttonTokens.slice(1), midTokens);
assert.deepEqual(midTokens.slice(1), [base, 'shared']);
assert.equal(new Set(buttonTokens).size, buttonTokens.length);
assert.ok({css:?}.includes('.' + buttonTokens[0]));
assert.ok({css:?}.includes('.' + midTokens[0]));
assert.ok({css:?}.includes('.' + base));
assert.equal(links.length, {mounts});
console.log('composed');
"#,
            mounts = if load == CssLoad::AutoMount { 2 } else { 0 }
        );
        assert_eq!(observe_named(artifacts, &check), b"composed\n");

        write(
            dir.root(),
            "main.mjs",
            "import './style.module.css'; import './global.css'; export const value = 1;",
        );
        let unused_mappings = session.build(&config).unwrap();
        assert_eq!(
            unused_mappings.product.artifacts.resources[stylesheet],
            artifacts.resources[stylesheet]
        );
        assert_eq!(
            observe_named(
                &unused_mappings.product.artifacts,
                &format!(
                    "{CSS_DOM_RECORDER}\nawait import('./main.mjs'); assert.equal(links.length, {}); console.log('unpruned');",
                    if load == CssLoad::AutoMount { 2 } else { 0 }
                )
            ),
            b"unpruned\n"
        );
        write(
            dir.root(),
            "main.mjs",
            "export { button, mid, base } from './style.module.css'; import './global.css';",
        );
    }
}

#[test]
fn css_modules_composition_errors_reject_unused_exports_under_both_loaders() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.module.css';");
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        write(dir.root(), "style.module.css", ".base {}");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let previous = session.build(&config).unwrap();
        for (source, message) in [
            (".a { composes: missing }", "unknown local name"),
            (".a { composes: a }", "composition cycle a -> a"),
            (
                ".a { composes: b } .b { composes: c } .c { composes: a }",
                "a -> b -> c -> a",
            ),
            (".a,.b { composes: a }", "multi-target"),
            (".a .b { composes: a }", "multi-target"),
            (".a:hover { composes: a }", "illegal placement"),
            (
                ":global(.a) { composes: a from global }",
                "illegal placement",
            ),
            (".a { & { composes: a } }", "illegal placement"),
            ("@font-face { composes: a }", "illegal placement"),
            (
                ".a { composes: b from './missing.module.css' }",
                "missing.module.css",
            ),
            (".a { composes: b !important }", "important composes"),
            (".a { composes: b from global extra }", "trailing composes"),
            (".a { composes: }", "empty composes"),
        ] {
            write(dir.root(), "style.module.css", source);
            let error = format!("{:#}", session.build(&config).err().unwrap());
            assert!(
                error.contains("style.module.css")
                    && error.contains(message)
                    && error.contains(".."),
                "{load:?} {source}: {error}"
            );
            assert!(Rc::ptr_eq(
                session.last.as_ref().unwrap(),
                &previous.product
            ));
        }
    }
}

#[test]
fn css_modules_cross_file_composition_is_lazy_shared_ordered_and_render_imports_stay_conditional() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = () => import('./lazy.mjs');",
    );
    write(
        dir.root(),
        "node_modules/styles/package.json",
        r#"{"exports":{"./base":"./base.module.css"}}"#,
    );
    write(
        dir.root(),
        "node_modules/styles/base.module.css",
        r#".b\61 se-name { color: green; background: url('./pixel.bin') }"#,
    );
    write(dir.root(), "node_modules/styles/pixel.bin", "pixel");
    write(
        dir.root(),
        "mid.module.css",
        r#".mid { composes: b\61 se-name from "styles/base" }"#,
    );
    write(
        dir.root(),
        "button.module.css",
        r#"@import './conditional.module.css' layer(theme) supports(display: grid) screen;
.button { composes: mid from "./m\69 d.module.css"; composes: mid from './mid.module.css'; color: red }"#,
    );
    write(
        dir.root(),
        "peer.module.css",
        r#".peer { composes: base-name from "styles/base" }"#,
    );
    write(
        dir.root(),
        "conditional.module.css",
        ".render { composes: support from './render-only.module.css' }",
    );
    write(
        dir.root(),
        "render-only.module.css",
        "@import './conditional.module.css' print; .support { color: blue }",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    config.entries.get_mut("main").unwrap().output_name = Some("deep/main.mjs".into());
    let mut resources = None;
    for reverse in [false, true] {
        write(
            dir.root(),
            "lazy.mjs",
            if reverse {
                "export {peer} from './peer.module.css'; export {button} from './button.module.css';"
            } else {
                "export {button} from './button.module.css'; export {peer} from './peer.module.css';"
            },
        );
        for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
            config.css.load = load;
            let mut session = BuildSession::new(dir.root()).unwrap();
            let built = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(
                &built.product,
                &session.build(&config).unwrap().product
            ));
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(disk.counters.source_parses, 0);
            assert_eq!(built.product.artifacts, disk.product.artifacts);
            let artifacts = &built.product.artifacts;
            assert_eq!(artifacts.stylesheets.len(), 4);
            assert_eq!(artifacts.resources.len(), 7);
            for path in ["conditional.module.css", "render-only.module.css"] {
                assert!(built
                    .product
                    .graph
                    .file_id(&dir.root().join(path))
                    .is_none());
            }
            let button_css = std::str::from_utf8(
                &artifacts.resources[&artifacts.stylesheets[&dir.root().join("button.module.css")]],
            )
            .unwrap();
            assert!(button_css.contains("layer(theme) supports(display: grid) screen;"));
            assert!(!button_css.contains("composes"));
            if let Some(previous) = &resources {
                assert_eq!(previous, &artifacts.resources);
            }
            resources = Some(artifacts.resources.clone());
            let ordered = if reverse {
                [
                    "node_modules/styles/base.module.css",
                    "peer.module.css",
                    "mid.module.css",
                    "button.module.css",
                ]
            } else {
                [
                    "node_modules/styles/base.module.css",
                    "mid.module.css",
                    "button.module.css",
                    "peer.module.css",
                ]
            };
            let ordered: Vec<_> = ordered
                .into_iter()
                .map(|path| {
                    artifacts.stylesheets[&dir.root().join(path)]
                        .to_str()
                        .unwrap()
                })
                .collect();
            let check = format!(
                r#"{CSS_DOM_RECORDER}
const main = await import('./deep/main.mjs');
assert.equal(links.length, 0);
const result = await main.load();
assert.equal(result.button.split(' ').length, 3);
assert.equal(result.peer.split(' ').length, 2);
assert.equal(result.button.split(' ')[2], result.peer.split(' ')[1]);
assert.equal(new Set(result.button.split(' ')).size, 3);
assert.strictEqual(await main.load(), result);
assert.equal(links.length, {mounts});
if ({mounts}) assert.deepEqual(links.map(link => link.href), {ordered:?}.map(path => new URL(path, import.meta.url).href));
for (const link of links) assert.ok(existsSync(new URL(link.href)));
console.log('cross-file');
"#,
                mounts = if load == CssLoad::AutoMount { 4 } else { 0 },
            );
            assert_eq!(observe_named(artifacts, &check), b"cross-file\n");
        }
    }
}

#[test]
fn css_modules_cross_file_composition_rechecks_closure_and_invalidates_reverse_users() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {button} from './button.module.css';",
    );
    write(
        dir.root(),
        "button.module.css",
        ".button { composes: mid from './mid.module.css' }",
    );
    write(
        dir.root(),
        "mid.module.css",
        ".mid { composes: base from './base.module.css' }",
    );
    write(dir.root(), "imported.css", ".global { color: red }");
    write(dir.root(), "pixel.bin", "one");
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let base = "@import './imported.css'; .base { composes: old from global; background: url('./pixel.bin') }";
        write(dir.root(), "base.module.css", base);
        write(dir.root(), "pixel.bin", "one");
        write(dir.root(), "imported.css", ".global { color: red }");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        let consumer = dir.root().join("button.module.css");
        let before = session.sources[&consumer].record.source_hash;
        write(dir.root(), "base.module.css", &base.replace("old", "new"));
        let changed = session.build(&config).unwrap();
        assert_ne!(before, session.sources[&consumer].record.source_hash);
        assert!(session.sources[&consumer].text.contains(" new"));
        assert_eq!(changed.counters.source_parses, 3);
        for path in [
            "main.mjs",
            "button.module.css",
            "mid.module.css",
            "base.module.css",
        ] {
            assert!(
                changed.invalidated.contains(&dir.root().join(path)),
                "{path}"
            );
        }
        assert_ne!(
            cold.product.artifacts.stylesheets,
            changed.product.artifacts.stylesheets
        );
        let mapping_hash = session.sources[&consumer].record.source_hash;
        for (path, text) in [
            ("pixel.bin", "two"),
            ("imported.css", ".global { color: blue }"),
        ] {
            write(dir.root(), path, text);
            let resource_edit = session.build(&config).unwrap();
            assert_eq!(
                resource_edit.counters.source_parses,
                if load == CssLoad::CallerLoaded { 0 } else { 3 }
            );
            if load == CssLoad::CallerLoaded {
                assert_eq!(mapping_hash, session.sources[&consumer].record.source_hash);
            }
            for path in [
                "main.mjs",
                "button.module.css",
                "mid.module.css",
                "base.module.css",
            ] {
                assert!(
                    resource_edit.invalidated.contains(&dir.root().join(path)),
                    "{path}"
                );
            }
            assert_ne!(
                changed.product.artifacts.stylesheets,
                resource_edit.product.artifacts.stylesheets
            );
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(resource_edit.product.artifacts, disk.product.artifacts);
            assert_eq!(disk.counters.source_parses, 0);
        }
        let last = session.last.clone().unwrap();
        fs::remove_file(dir.root().join("base.module.css")).unwrap();
        let error = format!("{:#}", session.build(&config).err().unwrap());
        assert!(
            error.contains("base.module.css") && error.contains(".."),
            "{error}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &last));
        write(dir.root(), "base.module.css", &base.replace("old", "new"));
        fs::remove_file(dir.root().join("pixel.bin")).unwrap();
        assert!(session.build(&config).is_err());
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &last));
    }
}

#[test]
fn css_modules_cross_file_composition_hashes_resolved_edges_not_only_closure_members() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {button} from './button.module.css';",
    );
    write(
        dir.root(),
        "button.module.css",
        "@import './node_modules/pkg/a.module.css' print; @import './node_modules/pkg/b.module.css' print; .button { composes: base from 'pkg' }",
    );
    for path in ["a.module.css", "b.module.css"] {
        write(dir.root(), &format!("node_modules/pkg/{path}"), ".base {}");
    }
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut previous = None;
    for target in ["a", "b"] {
        write(
            dir.root(),
            "node_modules/pkg/package.json",
            &format!(r#"{{"exports":{{".":"./{target}.module.css"}}}}"#),
        );
        let built = session.build(&config).unwrap();
        let consumer = dir.root().join("button.module.css");
        let identity = built.product.artifacts.stylesheets[&consumer].clone();
        let mapping = session.sources[&consumer].record.source_hash;
        if let Some((previous_identity, previous_mapping)) = previous {
            assert_ne!(identity, previous_identity);
            assert_ne!(mapping, previous_mapping);
            assert!(built.invalidated.contains(&consumer));
            assert!(built.invalidated.contains(&dir.root().join("main.mjs")));
        }
        previous = Some((identity, mapping));
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(disk.product.artifacts, built.product.artifacts);
        assert_eq!(disk.counters.source_parses, 0);
    }
}

#[test]
fn css_modules_everything_rejects_non_css_composition_targets_under_both_loaders() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './button.css';");
    write(
        dir.root(),
        "button.css",
        ".button { composes: base from './base.txt' }",
    );
    write(dir.root(), "base.txt", ".base { color: red }");
    let mut config = options();
    config.css.css_modules = CssModules::Everything;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        let error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("non-module composition target")
                && error.contains("button.css")
                && error.contains("base.txt"),
            "{load:?}: {error}"
        );
    }
}

#[test]
fn css_modules_cross_file_composition_rejects_nonlocal_targets_and_file_cycles() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './button.module.css';");
    write(dir.root(), "global.css", ".base {}");
    write(
        dir.root(),
        "private.module.css",
        ":global(.base) {} .base-name {}",
    );
    write(dir.root(), "module.mjs", "export const base = 'base';");
    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"exports":{".":"./index.module.css"}}"#,
    );
    write(dir.root(), "node_modules/pkg/index.module.css", ".base {}");
    write(
        dir.root(),
        "node_modules/pkg/private.module.css",
        ".base {}",
    );
    write(
        dir.root(),
        "a.module.css",
        ".a { composes: b from './b.module.css' }",
    );
    write(
        dir.root(),
        "b.module.css",
        ".b { composes: a from './a.module.css' }",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    config.externals.insert("external".into());
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        for (name, request, message) in [
            ("base", "./global.css", "non-module composition target"),
            ("base", "./module.mjs", "non-module composition target"),
            (
                "base",
                "./private.module.css",
                "missing/private exported local mapping",
            ),
            (
                "baseName",
                "./private.module.css",
                "missing/private exported local mapping",
            ),
            ("base", "pkg/private.module.css", "composes request"),
            ("base", "external", "target must be a local CSS module"),
            ("a", "./a.module.css", "cross-file composition cycle"),
            ("base", "./missing.module.css", "composes request"),
        ] {
            write(
                dir.root(),
                "button.module.css",
                &format!(".button {{ composes: {name} from '{request}' }}"),
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
                error.contains(message) && error.contains(".module.css") && error.contains(".."),
                "{load:?}: {error}"
            );
        }
    }
}

#[test]
fn css_modules_cross_file_composition_rejects_outside_inventory_and_symlinks() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    write(&root, "main.mjs", "import './button.module.css';");
    write(dir.root(), "outside.module.css", ".base {}");
    write(&root, ".bundler-hidden/base.module.css", ".base {}");
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    for request in ["../outside.module.css", "./.bundler-hidden/base.module.css"] {
        write(
            &root,
            "button.module.css",
            &format!(".button {{ composes: base from '{request}' }}"),
        );
        let error = format!(
            "{:#}",
            BuildSession::new(&root)
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("outside bounded source inventory") && error.contains(".."),
            "{error}"
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("../outside.module.css", root.join("alias.module.css")).unwrap();
        write(
            &root,
            "button.module.css",
            ".button { composes: base from './alias.module.css' }",
        );
        let error = format!(
            "{:#}",
            BuildSession::new(&root)
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("symlinks in build inventory unsupported"),
            "{error}"
        );
    }
}

#[test]
fn css_modules_stability_content_order_root_and_transitive_selection_invalidate_resources() {
    let mut original = None;
    for reverse in [false, true] {
        let dir = TmpDir::new();
        write(dir.root(), "a.module.css", ".same {color:red}");
        write(dir.root(), "b.module.css", ".same {color:blue}");
        write(
            dir.root(),
            "parent.css",
            "@import './a.module.css'; .global {}",
        );
        write(
            dir.root(),
            "main.mjs",
            if reverse {
                "export {same as b} from './b.module.css'; export {same as a} from './a.module.css'; import './parent.css';"
            } else {
                "export {same as a} from './a.module.css'; export {same as b} from './b.module.css'; import './parent.css';"
            },
        );
        let mut config = options();
        config.css.css_modules = CssModules::Enabled;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        let observation = observe_named(&cold.product.artifacts, "import {a,b} from './main.mjs'; if(a===b) throw new Error('aliased files'); console.log(JSON.stringify([a,b]));");
        if let Some(original) = &original {
            assert_eq!(&observation, original);
        } else {
            original = Some(observation.clone());
        }
        let before = session.sources[&dir.root().join("a.module.css")]
            .text
            .clone();
        let parent = cold.product.artifacts.stylesheets[&dir.root().join("parent.css")].clone();
        write(dir.root(), "a.module.css", ".same {color:green}");
        let edited = session.build(&config).unwrap();
        assert_eq!(
            session.sources[&dir.root().join("a.module.css")].text,
            before
        );
        assert_ne!(
            edited.product.artifacts.stylesheets[&dir.root().join("parent.css")],
            parent
        );
        assert_eq!(
            observe_named(
                &edited.product.artifacts,
                "import {a,b} from './main.mjs'; console.log(JSON.stringify([a,b]));"
            ),
            observation
        );
        config.css.css_modules = CssModules::Disabled;
        // The named JS imports are now invalid, but the resource closure alone
        // must still distinguish global output from the transformed version.
        config.entries.get_mut("main").unwrap().path = "parent.css".into();
        let global = session.build(&config).unwrap();
        assert_ne!(
            global.product.artifacts.stylesheets[&dir.root().join("parent.css")],
            edited.product.artifacts.stylesheets[&dir.root().join("parent.css")]
        );
        assert!(global
            .product
            .artifacts
            .resources
            .values()
            .any(|bytes| bytes == b".same {color:green}"));
    }
}

#[test]
fn css_modules_export_collision_has_both_spans_and_preserves_last_product() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {fooBar} from './style.module.css';",
    );
    write(dir.root(), "style.module.css", ".foo-bar {}");
    let mut config = options();
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    write(dir.root(), "style.module.css", ".foo-bar, #fooBar {}");
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("style.module.css")
            && error.contains("export collision")
            && error.contains("foo-bar")
            && error.contains("fooBar")
            && error.contains("1..8")
            && error.contains("11..17"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &built.product));
}

#[test]
fn caller_loaded_css_preserves_default_artifacts_across_memory_and_disk_hits() {
    let dir = TmpDir::new();
    write(dir.root(), "style.css", ".button { color: red; }\n");
    write(
        dir.root(),
        "main.cjs",
        "module.exports = require('./style.css');",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "style.css".into();
    config.entries.insert(
        "require".into(),
        Entry {
            path: "main.cjs".into(),
            output_name: Some("require.cjs".into()),
            namespace: true,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let path = dir.root().join("style.css");
    let artifact = &cold.product.artifacts.stylesheets[&path];
    let expected =
        crate::resource::url_module(artifact, &crate::output_url::PublicBase::new("./").unwrap())
            .unwrap();
    assert_eq!(session.sources[&path].text, expected);
    assert_eq!(
        cold.product.artifacts.entries["main"],
        Path::new("style.cjs")
    );
    assert_eq!(cold.product.artifacts.resources.len(), 1);
    assert_eq!(
        cold.product.artifacts.resources[artifact],
        b".button { color: red; }\n"
    );
    assert_eq!(cold.counters.source_parses, 2);
    assert!(session.sources[&path].record.config.commonjs);
    assert_eq!(
        observe_named(
            &cold.product.artifacts,
            "import css from './style.cjs'; import required from './require.cjs'; console.log(css === required);"
        ),
        b"true\n"
    );
    config.css = serde_json::from_str(r#"{"load":"caller-loaded"}"#).unwrap();
    let memory = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&memory.product, &cold.product));
    assert_eq!(memory.counters.memory_hits, 2);
    assert_eq!(memory.counters.output_parses, 0);
    session
        .sources
        .get_mut(&path)
        .unwrap()
        .record
        .config
        .commonjs = false;
    let mismatched_memory = session.build(&config).unwrap();
    assert_eq!(mismatched_memory.counters.memory_hits, 1);
    assert_eq!(mismatched_memory.counters.disk_hits, 1);
    assert_eq!(mismatched_memory.counters.source_parses, 0);
    assert_eq!(mismatched_memory.product.artifacts, cold.product.artifacts);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.disk_hits, 2);
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.source_reparses, 0);
    assert_eq!(disk.counters.fragment_reparses, 0);
    assert_eq!(disk.product.artifacts, cold.product.artifacts);
    assert_eq!(disk.product.fingerprints, cold.product.fingerprints);
    assert_eq!(
        serde_json::from_str::<CssOptions>(r#"{"load":"auto-mount"}"#)
            .unwrap()
            .load,
        CssLoad::AutoMount
    );
}

const CSS_DOM_RECORDER: &str = r#"
import assert from 'node:assert/strict';
import {existsSync} from 'node:fs';
const links = [];
globalThis.document = {
    URL: 'https://document.invalid/not/the/build/root/',
    baseURI: 'https://document.invalid/not/the/build/root/',
    createElement(tag) {
        assert.equal(tag, 'link');
        return {};
    },
    head: {appendChild(link) {
        assert.equal(link.rel, 'stylesheet');
        assert.equal(typeof link.href, 'string');
        links.push(link);
    }}
};
"#;

#[test]
fn auto_mount_css_no_document_throws_even_with_unused_url_and_pure_dom_contracts() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import unused from './style.css';");
    write(dir.root(), "style.css", ".a { color: red; }");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.declarations.user.push(ecco::parse_declarations(
        "global fn document.createElement(tag) {call_effects = false returns.*_effects = false}\n\
         global fn document.head.appendChild(link) {call_effects = false returns.*_effects = false}"
    ).unwrap());
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            r#"
try {
    await import('./main.mjs');
    throw new Error('unexpected success');
} catch (error) {
    if (error.message !== 'CSS auto-mount requires document') throw error;
    console.log(error.message);
}
"#
        ),
        b"CSS auto-mount requires document\n"
    );
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            &format!(
                "{CSS_DOM_RECORDER}
await import('./main.mjs');
assert.equal(links.length, 1);
assert.ok(existsSync(new URL(links[0].href)));
console.log('mounted');
"
            )
        ),
        b"mounted\n"
    );
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            r#"
globalThis.document = {createElement() {return {}}};
try {await import('./main.mjs'); throw new Error('unexpected success');}
catch (error) {if (!(error instanceof TypeError)) throw error; console.log('missing head');}
"#
        ),
        b"missing head\n"
    );
}

#[test]
fn auto_mount_css_is_synchronous_ordered_once_per_native_evaluation_and_cache_safe() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import first from './a.css';
import './between.mjs';
import second from './b.css';
if (document.links.length !== 2) throw new Error('mount was not synchronous');
export {first, second};
"#,
    );
    write(
        dir.root(),
        "between.mjs",
        "if (document.links.length !== 1) throw new Error('wrong dependency order');",
    );
    write(dir.root(), "a.css", ".a {color: red}");
    write(dir.root(), "b.css", ".b {color: blue}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("deep/app/main.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let memory = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&memory.product, &cold.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.product.artifacts, cold.product.artifacts);
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.source_reparses, 0);
    assert_eq!(disk.counters.fragment_reparses, 0);
    let a = &cold.product.artifacts.stylesheets[&dir.root().join("a.css")];
    let b = &cold.product.artifacts.stylesheets[&dir.root().join("b.css")];
    let check = format!(
        r#"{CSS_DOM_RECORDER}
document.links = links;
const first = await import('./deep/app/main.mjs');
const again = await import('./deep/app/main.mjs');
assert.strictEqual(first, again);
assert.deepEqual(links.map(link => link.href), [first.first, first.second]);
assert.equal(first.first, new URL({a:?}, import.meta.url).href);
assert.equal(first.second, new URL({b:?}, import.meta.url).href);
for (const link of links) assert.ok(existsSync(new URL(link.href)));
console.log('ordered once');
"#,
        a = a.to_str().unwrap(),
        b = b.to_str().unwrap()
    );
    assert_eq!(
        observe_named(&cold.product.artifacts, &check),
        b"ordered once\n"
    );
}

#[test]
fn auto_mount_css_lazy_and_dead_imports_do_not_mount_or_emit_early() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
export const load = () => import('./lazy.mjs');
function dead() {return import('./dead.css');}
"#,
    );
    write(
        dir.root(),
        "lazy.mjs",
        "export {default} from './live.css';",
    );
    write(dir.root(), "live.css", ".live {}");
    write(dir.root(), "dead.css", ".dead {}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(built.product.artifacts.resources.len(), 1);
    assert!(!built
        .product
        .artifacts
        .stylesheets
        .contains_key(&dir.root().join("dead.css")));
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            &format!(
                r#"{CSS_DOM_RECORDER}
const main = await import('./nested/main.mjs');
assert.equal(links.length, 0);
const lazy = await main.load();
assert.equal(links.length, 1);
assert.equal(lazy.default, links[0].href);
assert.ok(existsSync(new URL(lazy.default)));
assert.strictEqual(await main.load(), lazy);
assert.equal(links.length, 1);
console.log('lazy once');
"#
            )
        ),
        b"lazy once\n"
    );
}

#[test]
fn auto_mount_css_deferred_require_waits_for_call_and_returns_native_default() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as load} from './loader.cjs';",
    );
    write(
        dir.root(),
        "loader.cjs",
        "module.exports = () => require('./style.css');",
    );
    write(dir.root(), "style.css", ".a {}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            r#"
const {load} = await import('./main.mjs');
try {load(); throw new Error('unexpected success');}
catch (error) {
    if (error.message !== 'CSS auto-mount requires document') throw error;
    console.log('deferred error');
}
"#
        ),
        b"deferred error\n"
    );
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            &format!(
                r#"{CSS_DOM_RECORDER}
const main = await import('./main.mjs');
assert.equal(links.length, 0);
const css = main.load();
assert.equal(links.length, 1);
assert.equal(css.default, links[0].href);
assert.ok(existsSync(new URL(css.default)));
assert.strictEqual(main.load(), css);
assert.equal(links.length, 1);
console.log('deferred once');
"#
            )
        ),
        b"deferred once\n"
    );
}

#[test]
fn auto_mount_css_bases_follow_final_nested_entry_owner_not_alias_or_document() {
    let dir = TmpDir::new();
    write(dir.root(), "source/style.css", ".a {}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().path = "source/style.css".into();
    // The first public name owns the body; the other is only a namespace alias.
    config.entries.get_mut("main").unwrap().output_name = Some("deep/canonical/style.mjs".into());
    config.entries.insert(
        "z-alias".into(),
        Entry {
            path: "source/style.css".into(),
            output_name: Some("other/alias.mjs".into()),
            namespace: true,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    for base in [
        "./",
        "./public space/",
        "../outside/",
        "/static/",
        "https://cdn.example/path/",
    ] {
        config.output.base_url = base.into();
        let built = session.build(&config).unwrap();
        assert_eq!(
            built.product.artifacts.entries["main"],
            Path::new("deep/canonical/style.mjs")
        );
        let style = &built.product.artifacts.stylesheets[&dir.root().join("source/style.css")];
        let public = crate::output_url::PublicBase::new(base)
            .unwrap()
            .join(style)
            .unwrap();
        assert_eq!(
            observe_named(
                &built.product.artifacts,
                &format!(
                    r#"{CSS_DOM_RECORDER}
const alias = await import('./other/alias.mjs');
const canonical = await import('./deep/canonical/style.mjs');
assert.equal(typeof canonical.default, 'string');
assert.equal(alias.default, canonical.default);
assert.equal(links.length, 1);
assert.equal(links[0].href, canonical.default);
assert.equal(canonical.default, new URL({public:?}, import.meta.url).href);
console.log('correct base');
"#
                )
            ),
            b"correct base\n"
        );
    }
    config.output.base_url = "./".into();
    config.entries.clear();
    config.entries.insert(
        "main".into(),
        Entry {
            path: "source/style.css".into(),
            output_name: None,
            namespace: true,
        },
    );
    let default_name = session.build(&config).unwrap();
    assert_eq!(
        default_name.product.artifacts.entries["main"],
        Path::new("source/style.mjs")
    );
    assert_eq!(
        observe_named(
            &default_name.product.artifacts,
            &format!(
                r#"{CSS_DOM_RECORDER}
const css = await import('./source/style.mjs');
assert.equal(links.length, 1);
assert.equal(links[0].href, css.default);
assert.ok(existsSync(new URL(css.default)));
console.log('esm entry');
"#
            )
        ),
        b"esm entry\n"
    );
}

#[test]
fn css_modules_custom_suffix_aliases_keep_owned_paths_and_mount_once_for_each_base() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "source/style.module.cjs",
        ".button-label {color: red}",
    );
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".module.cjs".into());
    config.entries.get_mut("main").unwrap().path = "source/style.module.cjs".into();
    config.entries.insert(
        "z-alias".into(),
        Entry {
            path: "source/style.module.cjs".into(),
            output_name: Some("deep/alias/styles.mjs".into()),
            namespace: true,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut owned = None;
    for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
        config.css.load = load;
        for base in [
            "./",
            "./public space/",
            "../outside/",
            "/static/",
            "http://cdn.example/styles/",
            "https://cdn.example/styles/",
        ] {
            config.output.base_url = base.into();
            let built = session.build(&config).unwrap();
            let artifacts = &built.product.artifacts;
            assert_eq!(
                artifacts.entries["main"],
                Path::new("source/style.module.mjs")
            );
            assert_eq!(artifacts.stylesheets.len(), 1);
            assert_eq!(artifacts.resources.len(), 1);
            if let Some(previous) = &owned {
                assert_eq!(previous, &artifacts.resources);
            }
            owned = Some(artifacts.resources.clone());
            let stylesheet = &artifacts.stylesheets[&dir.root().join("source/style.module.cjs")];
            assert!(artifacts.resources.contains_key(stylesheet));
            assert!(!artifacts.files.contains_key(stylesheet));
            let public = crate::output_url::PublicBase::new(base)
                .unwrap()
                .join(stylesheet)
                .unwrap();
            assert_eq!(
                observe_named(
                    artifacts,
                    &format!(
                        r#"{CSS_DOM_RECORDER}
const alias = await import('./deep/alias/styles.mjs');
const canonical = await import('./source/style.module.mjs');
assert.deepEqual(Object.keys(alias), ['buttonLabel']);
assert.deepEqual(Object.keys(canonical), ['buttonLabel']);
assert.equal(alias.buttonLabel, canonical.buttonLabel);
assert.match(canonical.buttonLabel, /^_[A-Za-z0-9_-]{{10}}$/);
assert.strictEqual(await import('./deep/alias/styles.mjs'), alias);
assert.equal(links.length, {mounts});
if ({mounts}) assert.equal(links[0].href, new URL({public:?}, import.meta.url).href);
console.log('module alias');
"#,
                        mounts = usize::from(load == CssLoad::AutoMount),
                    ),
                ),
                b"module alias\n",
            );
        }
    }
}

#[test]
fn auto_mount_css_http_origin_and_cdn_urls_use_native_owner_meta_url() {
    let dir = TmpDir::new();
    write(dir.root(), "style.css", ".a {}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().path = "style.css".into();
    config.entries.get_mut("main").unwrap().output_name = Some("deep/styles/main.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    for base in ["./", "./public/", "/static/", "https://cdn.example/static/"] {
        config.output.base_url = base.into();
        let built = session.build(&config).unwrap();
        let artifacts = &built.product.artifacts;
        let style = &artifacts.stylesheets[&dir.root().join("style.css")];
        let public = crate::output_url::PublicBase::new(base)
            .unwrap()
            .join(style)
            .unwrap();
        let expected = url::Url::parse("https://app.example/generation-42/")
            .unwrap()
            .join(&public)
            .unwrap();
        let runner = format!(
            r#"
import assert from 'node:assert/strict';
import vm from 'node:vm';
const links = [];
const context = vm.createContext({{URL, document: {{
    baseURI: 'https://wrong.example/document/',
    createElement() {{return {{}};}},
    head: {{appendChild(link) {{links.push(link);}}}}
}}}});
const files = {files};
const modules = new Map();
function load(id) {{
    if (modules.has(id)) return modules.get(id);
    const path = new URL(id).pathname.slice('/generation-42/'.length);
    assert.ok(Object.hasOwn(files, path), path);
    const module = new vm.SourceTextModule(files[path], {{
        context, identifier: id, initializeImportMeta(meta) {{meta.url = id;}}
    }});
    modules.set(id, module);
    return module;
}}
const entry = load('https://app.example/generation-42/deep/styles/main.mjs');
await entry.link((request, importer) => load(new URL(request, importer.identifier).href));
await entry.evaluate();
assert.equal(entry.namespace.default, {expected:?});
assert.equal(links.length, 1);
assert.equal(links[0].href, {expected:?});
console.log('http origin');
"#,
            files = serde_json::to_string(&artifacts.files).unwrap(),
            expected = expected.as_str()
        );
        write(dir.root(), "http-check.mjs", &runner);
        let output = std::process::Command::new("node")
            .arg("--experimental-vm-modules")
            .arg(dir.root().join("http-check.mjs"))
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"http origin\n");
    }
}

#[test]
fn css_modules_published_shared_static_dynamic_and_cjs_activation_keep_generations() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {sharedName} from './shared.style'; \
         export const load = () => import('./nested/lazy.mjs'); \
         export {default as requireStyle} from './loader.cjs';",
    );
    write(
        dir.root(),
        "other.mjs",
        "export {sharedName} from './shared.style';",
    );
    write(
        dir.root(),
        "nested/lazy.mjs",
        "export {sharedName} from '../shared.style'; \
         export {buttonLabel} from '../button.style'; \
         export const deeper = () => import('./deeper.mjs');",
    );
    write(
        dir.root(),
        "nested/deeper.mjs",
        "export {deepName} from '../deep.style';",
    );
    write(
        dir.root(),
        "loader.cjs",
        "module.exports = () => require('./required.style');",
    );
    let shared = "@import './theme.css' screen; \
                  .shared-name {color: red; background: url('./pixel.bin?v=1#icon')}";
    write(dir.root(), "shared.style", shared);
    write(dir.root(), "theme.css", ".theme {color: yellow}");
    write(dir.root(), "pixel.bin", "first pixel");
    write(
        dir.root(),
        "button.style",
        ".button-label {composes: shared-name from './shared.style'; color: red}",
    );
    write(dir.root(), "deep.style", ".deep-name {color: green}");
    write(dir.root(), "required.style", ".required-name {color: blue}");
    let mut config = options();
    config.css.css_modules = CssModules::Extension(".style".into());
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("deep/app/main.mjs".into());
    for (name, path, output) in [
        ("other", "other.mjs", "another/entry.mjs"),
        ("z-alias", "main.mjs", "aliases/main.mjs"),
    ] {
        config.entries.insert(
            name.into(),
            Entry {
                path: path.into(),
                output_name: Some(output.into()),
                namespace: true,
            },
        );
    }
    let mut session = BuildSession::new(dir.root()).unwrap();
    let output = dir.root().join(".bundler-published");
    let cancel = crate::publish::Cancellation::default();
    let (old_build, old) = session
        .build_and_publish(&config, &output, &cancel)
        .unwrap();
    let before = crate::publish::current_directory(&output).unwrap();
    let old_bytes: BTreeMap<_, _> = old_build
        .product
        .artifacts
        .file_bytes()
        .map(|(path, bytes)| (path.clone(), bytes.to_vec()))
        .collect();
    write(
        dir.root(),
        "shared.style",
        &shared.replace("color: red", "color: purple"),
    );
    write(dir.root(), "pixel.bin", "second pixel");
    let (new_build, new) = session
        .build_and_publish(&config, &output, &cancel)
        .unwrap();
    assert_ne!(before, new.directory);
    for (path, bytes) in &old_bytes {
        assert_eq!(&fs::read(old.directory.join(path)).unwrap(), bytes);
    }
    for (build, published) in [(&old_build, &old), (&new_build, &new)] {
        assert_eq!(build.product.artifacts.stylesheets.len(), 4);
        for path in build.product.artifacts.stylesheets.values() {
            assert!(published.directory.join(path).is_file());
            assert!(build.product.artifacts.resources.contains_key(path));
            assert!(!build.product.artifacts.files.contains_key(path));
        }
    }
    let urls = |published: &crate::publish::Published| {
        published
            .entries
            .iter()
            .map(|(name, path)| {
                (
                    name.clone(),
                    url::Url::from_file_path(path).unwrap().to_string(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let hrefs = |build: &BuildResult, published: &crate::publish::Published| {
        [
            "shared.style",
            "required.style",
            "button.style",
            "deep.style",
        ]
        .map(|input| {
            url::Url::from_file_path(
                published
                    .directory
                    .join(&build.product.artifacts.stylesheets[&dir.root().join(input)]),
            )
            .unwrap()
            .to_string()
        })
    };
    write(
        dir.root(),
        "published-modules-check.mjs",
        &format!(
            r#"{CSS_DOM_RECORDER}
import {{readFileSync}} from 'node:fs';
const generations = {generations};
const retained = [];
for (const [entries, expected] of generations) {{
    const start = links.length;
    const alias = await import(entries['z-alias']);
    const main = await import(entries.main);
    const other = await import(entries.other);
    assert.equal(links.length, start + 1);
    assert.equal(main.sharedName, alias.sharedName);
    assert.equal(main.sharedName, other.sharedName);
    assert.match(main.sharedName, /^_[A-Za-z0-9_-]{{10}}$/);
    assert.strictEqual(main.load, alias.load);
    assert.strictEqual(main.requireStyle, alias.requireStyle);
    const required = main.requireStyle();
    assert.deepEqual(Object.keys(required), ['requiredName']);
    assert.strictEqual(main.requireStyle(), required);
    assert.equal(links.length, start + 2);
    const lazy = await alias.load();
    assert.strictEqual(await main.load(), lazy);
    assert.equal(lazy.sharedName, main.sharedName);
    assert.equal(lazy.buttonLabel.split(' ')[1], main.sharedName);
    assert.equal(links.length, start + 3);
    const deep = await lazy.deeper();
    assert.deepEqual(Object.keys(deep), ['deepName']);
    assert.strictEqual(await lazy.deeper(), deep);
    assert.equal(links.length, start + 4);
    assert.deepEqual(links.slice(start).map(link => link.href), expected);
    for (const [index, mapping] of [main.sharedName, required.requiredName, lazy.buttonLabel, deep.deepName].entries()) {{
        assert.ok(existsSync(new URL(expected[index])));
        assert.ok(readFileSync(new URL(expected[index]), 'utf8').includes('.' + mapping.split(' ')[0]));
    }}
    const css = readFileSync(new URL(expected[0]), 'utf8');
    const generation = new URL('../../', entries.main).href;
    assert.ok(expected.every(href => href.startsWith(generation)));
    const references = [css.match(/@import "([^"]+)"/)[1], css.match(/url\("([^"]+)"\)/)[1]];
    for (const reference of references) {{
        const resolved = new URL(reference, expected[0]);
        assert.ok(resolved.href.startsWith(generation));
        assert.ok(existsSync(resolved), resolved.href);
    }}
    assert.match(css, /screen;/);
    assert.equal(readFileSync(new URL(references[1], expected[0]), 'utf8'), retained.length ? 'second pixel' : 'first pixel');
    retained.push({{main, lazy, deep, required}});
}}
assert.equal(retained[0].main.sharedName, retained[1].main.sharedName);
assert.equal(retained[0].lazy.buttonLabel, retained[1].lazy.buttonLabel);
assert.notEqual(generations[0][1][0], generations[1][1][0]);
assert.notEqual(generations[0][1][2], generations[1][1][2]);
assert.strictEqual(await retained[0].main.load(), retained[0].lazy);
assert.strictEqual(await retained[0].lazy.deeper(), retained[0].deep);
assert.strictEqual(retained[0].main.requireStyle(), retained[0].required);
assert.equal(links.length, 8);
console.log('published module activation');
"#,
            generations = serde_json::to_string(&[
                (urls(&old), hrefs(&old_build, &old)),
                (urls(&new), hrefs(&new_build, &new)),
            ])
            .unwrap(),
        ),
    );
    assert_eq!(
        observe_entry(&dir.root().join("published-modules-check.mjs")),
        b"published module activation\n"
    );
}

#[test]
fn auto_mount_css_published_lazy_generations_keep_their_own_resources_and_alias_identity() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = () => import('./lazy.mjs');",
    );
    write(
        dir.root(),
        "lazy.mjs",
        "export {default} from './style.css';",
    );
    write(dir.root(), "style.css", ".first {}");
    let mut config = options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("deep/main.mjs".into());
    config.entries.insert(
        "z-alias".into(),
        Entry {
            path: "main.mjs".into(),
            output_name: Some("elsewhere/alias.mjs".into()),
            namespace: true,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let output = dir.root().join(".bundler-published");
    let cancel = crate::publish::Cancellation::default();
    let (old_build, old) = session
        .build_and_publish(&config, &output, &cancel)
        .unwrap();
    write(dir.root(), "style.css", ".second {}");
    let (new_build, new) = session
        .build_and_publish(&config, &output, &cancel)
        .unwrap();
    assert_ne!(old.directory, new.directory);
    let old_style = &old_build.product.artifacts.stylesheets[&dir.root().join("style.css")];
    let new_style = &new_build.product.artifacts.stylesheets[&dir.root().join("style.css")];
    let old_url = url::Url::from_file_path(&old.entries["main"]).unwrap();
    let alias_url = url::Url::from_file_path(&old.entries["z-alias"]).unwrap();
    let new_url = url::Url::from_file_path(&new.entries["main"]).unwrap();
    let old_href = url::Url::from_file_path(old.directory.join(old_style)).unwrap();
    let new_href = url::Url::from_file_path(new.directory.join(new_style)).unwrap();
    write(
        dir.root(),
        "published-check.mjs",
        &format!(
            r#"{CSS_DOM_RECORDER}
const old = await import({old_url:?});
const alias = await import({alias_url:?});
const next = await import({new_url:?});
assert.equal(links.length, 0);
const first = await alias.load();
assert.strictEqual(await old.load(), first);
assert.equal(first.default, {old_href:?});
assert.equal(links.length, 1);
const second = await next.load();
assert.equal(second.default, {new_href:?});
assert.notEqual(first.default, second.default);
assert.deepEqual(links.map(link => link.href), [first.default, second.default]);
assert.ok(existsSync(new URL(first.default)));
assert.ok(existsSync(new URL(second.default)));
console.log('immutable generations');
"#,
            old_url = old_url.as_str(),
            alias_url = alias_url.as_str(),
            new_url = new_url.as_str(),
            old_href = old_href.as_str(),
            new_href = new_href.as_str()
        ),
    );
    assert_eq!(
        observe_entry(&dir.root().join("published-check.mjs")),
        b"immutable generations\n"
    );
}

#[test]
fn output_base_urls_cover_resource_exports_css_fetches_and_default_bytes() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as css} from './style.css'; export {default as image} from './image.png';",
    );
    write(dir.root(), "theme.css", ".theme { color: red; }\n");
    write(
        dir.root(),
        "style.css",
        "@import './theme.css' layer(theme);\n.a { background: url('./image.png?v=1#icon'); }\n\
.b { background: url('/original.png'); mask: url(https://origin.invalid/a); cursor: url(data:image/png,a), auto; }\n",
    );
    fs::write(dir.root().join("image.png"), [0, 255, 42]).unwrap();
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let default = session.build(&config).unwrap();
    let paths: BTreeSet<_> = default
        .product
        .artifacts
        .resources
        .keys()
        .cloned()
        .collect();
    let style = &default.product.artifacts.stylesheets[&dir.root().join("style.css")];
    let theme = paths
        .iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "css") && *path != style)
        .unwrap();
    let image = paths
        .iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "png"))
        .unwrap();
    let original = ".b { background: url('/original.png'); mask: url(https://origin.invalid/a); cursor: url(data:image/png,a), auto; }\n";
    assert_eq!(
        default.product.artifacts.resources[style],
        format!(
            "@import \"./{}\" layer(theme);\n.a {{ background: url(\"./{}?v=1#icon\"); }}\n{original}",
            theme.file_name().unwrap().to_str().unwrap(),
            image.file_name().unwrap().to_str().unwrap(),
        )
        .as_bytes()
    );
    assert_eq!(
        crate::resource::url_module(image, &crate::output_url::PublicBase::new("./").unwrap())
            .unwrap(),
        format!("module.exports = \"./{}\";\n", image.display())
    );
    for (base, normalized, relative) in [
        ("./", "./", true),
        ("/static", "/static/", false),
        ("/", "/", false),
        (
            "https://cdn.example/static",
            "https://cdn.example/static/",
            false,
        ),
        (
            "http://cdn.example:8080/static/",
            "http://cdn.example:8080/static/",
            false,
        ),
        ("./public", "./public/", true),
        ("../public/", "../public/", true),
        ("public/", "public/", true),
        ("./café a\"%20/", "./caf%C3%A9%20a%22%20/", true),
        ("/café a\"%20/", "/caf%C3%A9%20a%22%20/", false),
        (
            "https://cdn.example/café a\"%20 ",
            "https://cdn.example/caf%C3%A9%20a%22%20%20/",
            false,
        ),
    ] {
        config.output.base_url = base.into();
        let built = session.build(&config).unwrap();
        assert_eq!(
            built
                .product
                .artifacts
                .resources
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            paths,
            "{base}"
        );
        assert_eq!(
            built.product.artifacts.stylesheets,
            default.product.artifacts.stylesheets
        );
        assert_eq!(
            observe_named(
                &built.product.artifacts,
                "import {css, image} from './nested/main.mjs'; console.log(JSON.stringify([css, image]));"
            ),
            format!(
                "{}\n",
                serde_json::to_string(&[
                    format!("{normalized}{}", style.display()),
                    format!("{normalized}{}", image.display()),
                ]).unwrap()
            ).as_bytes(),
            "{base}"
        );
        let css = std::str::from_utf8(&built.product.artifacts.resources[style]).unwrap();
        let fetch = |path: &Path| {
            if relative {
                format!("./{}", path.file_name().unwrap().to_str().unwrap())
            } else {
                format!("{normalized}{}", path.display())
            }
        };
        assert_eq!(
            css,
            format!(
                "@import \"{}\" layer(theme);\n.a {{ background: url(\"{}?v=1#icon\"); }}\n{original}",
                fetch(theme), fetch(image)
            ),
            "{base}"
        );
        // Resolve actual browser-style fetches, not just the emitted text.
        let root = url::Url::parse("https://application.example/generation/").unwrap();
        let stylesheet_url = root
            .join(&format!("{normalized}{}", style.display()))
            .unwrap();
        for target in [theme, image] {
            assert_eq!(
                stylesheet_url.join(&fetch(target)).unwrap(),
                root.join(&format!("{normalized}{}", target.display()))
                    .unwrap(),
                "{base}"
            );
        }
        assert_eq!(built.counters.source_reparses, 0);
        assert_eq!(built.counters.fragment_reparses, 0);
    }
}

#[test]
fn output_base_edits_invalidate_generated_asset_and_css_sources_before_memory_hits() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as image} from './image.png'; export {default as css} from './style.css';",
    );
    write(
        dir.root(),
        "style.css",
        ".a { background: url('./image.png'); }",
    );
    fs::write(dir.root().join("image.png"), [255, 42]).unwrap();
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(cold.counters.source_parses, 3);
    config.output.base_url = "/static".into();
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.memory_hits, 1);
    assert_eq!(edited.counters.source_parses, 2);
    assert_ne!(
        cold.product.fingerprints.parse,
        edited.product.fingerprints.parse
    );
    assert_ne!(
        cold.product.fingerprints.output,
        edited.product.fingerprints.output
    );
    assert_eq!(
        cold.product.fingerprints.roots,
        edited.product.fingerprints.roots
    );
    assert!(edited.invalidated.contains(&dir.root().join("main.mjs")));
    assert_eq!(
        cold.product.artifacts.resources.keys().collect::<Vec<_>>(),
        edited
            .product
            .artifacts
            .resources
            .keys()
            .collect::<Vec<_>>()
    );
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.counters.disk_hits, 3);
    assert_eq!(warm.product.artifacts, edited.product.artifacts);
    config.output.base_url = "/static/".into();
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&noop.product, &edited.product));
    assert_eq!(noop.counters.memory_hits, 3);
    assert_eq!(noop.counters.output_parses, 0);
    config.output.base_url = "./".into();
    let restored = session.build(&config).unwrap();
    assert_eq!(restored.counters.memory_hits, 1);
    assert_eq!(restored.product.artifacts, cold.product.artifacts);
    assert_eq!(restored.product.fingerprints, cold.product.fingerprints);
}

#[test]
fn output_base_is_validated_even_without_resources_and_errors_preserve_last_product() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export const value = 1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut config = options();
    let cold = session.build(&config).unwrap();
    config.output.base_url = "/static/".into();
    let edited = session.build(&config).unwrap();
    assert_eq!(
        cold.product.fingerprints.parse,
        edited.product.fingerprints.parse
    );
    assert_ne!(
        cold.product.fingerprints.output,
        edited.product.fingerprints.output
    );
    assert_eq!(edited.counters.memory_hits, 1);
    assert_eq!(edited.counters.source_parses, 0);
    assert_eq!(cold.product.artifacts, edited.product.artifacts);
    for invalid in [
        "",
        "//cdn.example/a",
        "///cdn.example/a",
        "https:/cdn.example",
        "https:///a",
        "https://",
        "ftp://cdn.example/",
        "javascript:alert(1)",
        "data:text/plain,a",
        "https://user:password@cdn.example/",
        "https://@cdn.example/",
        "https://bad host/",
        "https://cdn.example:bad/",
        "https://[::1/",
        "/static?query",
        "./static#fragment",
        "./bad\\path",
        "./bad\npath",
        "./bad\tpath",
        "./bad\u{7f}path",
        "./bad%GG",
        "./bad%",
        "./bad%0a",
        "./bad%5c",
    ] {
        config.output.base_url = invalid.into();
        let error = session.build(&config).err().unwrap();
        assert!(
            format!("{error:#}").contains("output.base_url"),
            "{invalid:?}: {error:#}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &edited.product));
    }
}

#[test]
fn bare_string_package_exports_allow_nested_json_css_and_asset_modules() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"exports":"./index.mjs"}"#,
    );
    write(
        dir.root(),
        "node_modules/pkg/index.mjs",
        "export default 7;",
    );
    write(
        dir.root(),
        "node_modules/pkg/data/nested/value.json",
        r#"{"value":9}"#,
    );
    write(
        dir.root(),
        "node_modules/pkg/style/theme.css",
        ".theme { background: url('../assets/pixel.bin'); }",
    );
    write(dir.root(), "node_modules/pkg/assets/pixel.bin", "binary");
    write(
        dir.root(),
        "main.mjs",
        r#"
import root from 'pkg';
import data from 'pkg/data/nested/value.json';
import style from 'pkg/style/theme.css';
import image from 'pkg/assets/pixel.bin';
console.log(root, data.value, style.endsWith('.css'), image.endsWith('.bin'));
"#,
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&built.product.artifacts), b"7 9 true true\n");
    assert_eq!(built.product.artifacts.resources.len(), 2);
    assert_eq!(built.product.artifacts.stylesheets.len(), 1);
}

#[test]
fn css_feature_query_urls_are_not_dependencies_and_url_whitespace_is_normalized() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './style.css';");
    write(dir.root(), "theme.css", ".theme { color: green; }");
    write(
        dir.root(),
        "style.css",
        r#"
@import " https://example.invalid/theme.css ";
@import "./theme.css" supports(background-image: image-set(url("./probe.png") 1x));
@supports (background-image: image-set(url("./another-probe.png") 1x)) {
  body { background: url(" ./image.png "); }
}
.encoded { background: url("./%20padded.png%20"); }
"#,
    );
    fs::write(dir.root().join("image.png"), [1, 2]).unwrap();
    fs::write(dir.root().join(" padded.png "), [3, 4]).unwrap();
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(built.product.artifacts.resources.len(), 4);
    let style = built.product.artifacts.stylesheets.values().next().unwrap();
    let output = std::str::from_utf8(&built.product.artifacts.resources[style]).unwrap();
    for unchanged in [
        r#"@import " https://example.invalid/theme.css ";"#,
        r#"supports(background-image: image-set(url("./probe.png") 1x))"#,
        r#"@supports (background-image: image-set(url("./another-probe.png") 1x))"#,
    ] {
        assert!(output.contains(unchanged), "{output}");
    }
    for image in ["image.png", " padded.png "] {
        let path = dir.root().join(image);
        let artifact = crate::resource::asset_path(&path, &fs::read(&path).unwrap());
        assert!(built.product.artifacts.resources.contains_key(&artifact));
        assert!(
            output.contains(artifact.file_name().unwrap().to_str().unwrap()),
            "{output}"
        );
    }
}

#[test]
fn css_extraction_preserves_import_conditions_cycles_and_rewrites_local_urls() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as css} from './style.css';",
    );
    write(
        dir.root(),
        "style.css",
        r#"@charset "UTF-8";
@import "./nested/theme.css" layer(theme) supports(display: grid) screen;
@import url("https://example.invalid/remote.css") print;
@namespace svg url("urn:svg");
@namespace local url("./not-a-file-namespace");
/* url("missing.png") */
body {
  background: url("./logo%20file.png?v=1#icon");
  mask: url(#cutout);
  border-image: url("/public/border.png");
  content: "url(missing.png)";
  background-image: image-set("./logo%20file.png" 1x, url('./logo\ file.png') 2x);
  cursor: url("data:image/svg+xml,<svg/>"), auto;
}
"#,
    );
    write(
        dir.root(),
        "nested/theme.css",
        "@import '../style.css' layer(cycle);\n.theme { color: red; }\n",
    );
    fs::write(dir.root().join("logo file.png"), [255, 0, 42]).unwrap();
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(built.product.artifacts.stylesheets.len(), 1);
    assert_eq!(built.product.artifacts.resources.len(), 3);
    let style =
        &built.product.artifacts.stylesheets[&dir.root().join("style.css").canonicalize().unwrap()];
    let css = std::str::from_utf8(&built.product.artifacts.resources[style]).unwrap();
    let theme = built
        .product
        .artifacts
        .resources
        .keys()
        .find(|path| path.extension().is_some_and(|ext| ext == "css") && *path != style)
        .unwrap();
    let image = built
        .product
        .artifacts
        .resources
        .keys()
        .find(|path| path.extension().is_some_and(|ext| ext == "png"))
        .unwrap();
    assert!(css.starts_with("@charset \"UTF-8\";\n"));
    assert!(css.contains(&format!(
        "@import \"./{}\" layer(theme) supports(display: grid) screen;",
        theme.file_name().unwrap().to_str().unwrap()
    )));
    assert!(css.contains(&format!(
        "url(\"./{}?v=1#icon\")",
        image.file_name().unwrap().to_str().unwrap()
    )));
    assert!(css.contains("image-set(\"./"));
    for unchanged in [
        "@import url(\"https://example.invalid/remote.css\") print;",
        "@namespace local url(\"./not-a-file-namespace\");",
        "/* url(\"missing.png\") */",
        "mask: url(#cutout)",
        "border-image: url(\"/public/border.png\")",
        "content: \"url(missing.png)\"",
        "url(\"data:image/svg+xml,<svg/>\")",
    ] {
        assert!(css.contains(unchanged), "{css}");
    }
    let child = std::str::from_utf8(&built.product.artifacts.resources[theme]).unwrap();
    assert!(child.contains(&format!(
        "@import \"./{}\" layer(cycle);",
        style.file_name().unwrap().to_str().unwrap()
    )));
    assert_eq!(
        observe_named(
            &built.product.artifacts,
            "import {css} from './nested/main.mjs'; console.log(css);"
        ),
        format!("./{}\n", style.display()).as_bytes()
    );
    assert_eq!(built.counters.source_reparses, 0);
    assert_eq!(built.counters.fragment_reparses, 0);
    let output = TmpDir::new();
    let published = crate::publish::publish(
        output.root(),
        &built.product.artifacts,
        &crate::publish::Cancellation::default(),
    )
    .unwrap();
    for (path, bytes) in &built.product.artifacts.resources {
        assert_eq!(&fs::read(published.directory.join(path)).unwrap(), bytes);
    }
}

#[test]
fn css_transitive_edits_invalidate_parent_urls_and_failures_preserve_last_product() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './style.css'; console.log('main');",
    );
    write(
        dir.root(),
        "style.css",
        "@import './theme.css'; .a { background: url('./image.png'); }",
    );
    write(dir.root(), "theme.css", ".theme { color: red; }");
    fs::write(dir.root().join("image.png"), [255, 0]).unwrap();
    let config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.product.artifacts, cold.product.artifacts);
    fs::write(dir.root().join("image.png"), [254, 1]).unwrap();
    let image_edit = session.build(&config).unwrap();
    assert_ne!(
        cold.product.artifacts.stylesheets,
        image_edit.product.artifacts.stylesheets
    );
    assert_eq!(image_edit.counters.source_parses, 1);
    assert!(image_edit
        .invalidated
        .contains(&dir.root().join("main.mjs").canonicalize().unwrap()));
    write(dir.root(), "theme.css", ".theme { color: blue; }");
    let theme_edit = session.build(&config).unwrap();
    assert_ne!(
        image_edit.product.artifacts.stylesheets,
        theme_edit.product.artifacts.stylesheets
    );
    assert_eq!(observe(&theme_edit.product.artifacts), b"main\n");
    fs::remove_file(dir.root().join("image.png")).unwrap();
    assert!(format!("{:#}", session.build(&config).err().unwrap()).contains("CSS URL"));
    assert!(Rc::ptr_eq(
        session.last.as_ref().unwrap(),
        &theme_edit.product
    ));
}

#[test]
fn css_entries_are_extracted_and_invalid_or_escaping_urls_are_errors() {
    let dir = TmpDir::new();
    let project = dir.root().join("project");
    fs::create_dir(&project).unwrap();
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "style.css".into();
    write(&project, "style.css", ".a { color: red; }");
    let built = BuildSession::new(&project).unwrap().build(&config).unwrap();
    assert_eq!(
        built.product.artifacts.entries["main"],
        PathBuf::from("style.cjs")
    );
    assert_eq!(built.product.artifacts.stylesheets.len(), 1);
    fs::write(dir.root().join("outside.png"), [255]).unwrap();
    for css in [
        ".a { background: url(bad url); }".to_owned(),
        ".a { background: url('./bad%GG.png'); }".to_owned(),
        ".a { background: url('../outside.png'); }".to_owned(),
    ] {
        write(&project, "style.css", &css);
        assert!(
            BuildSession::new(&project).unwrap().build(&config).is_err(),
            "{css}"
        );
    }
}

#[test]
fn binary_assets_preserve_bytes_urls_cache_and_published_generations() {
    let dir = TmpDir::new();
    let original = [0, 255, 128, 10, 13, 42];
    fs::write(dir.root().join("image.png"), original).unwrap();
    fs::write(dir.root().join("duplicate.png"), original).unwrap();
    write(
        dir.root(),
        "loader.cjs",
        "exports.load = () => require('./image.png');",
    );
    write(
        dir.root(),
        "main.mjs",
        r#"
import image from './image.png';
import duplicate from './duplicate.png';
import loader from './loader.cjs';
export {image, duplicate};
export const same = () => image === loader.load();
export const dynamic = () => import('./image.png');
"#,
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(cold.product.artifacts.resources.len(), 1);
    let (asset, bytes) = cold.product.artifacts.resources.first_key_value().unwrap();
    assert_eq!(bytes.as_slice(), original);
    assert_eq!(
        observe_named(
            &cold.product.artifacts,
            r#"
import {image, duplicate, same, dynamic} from './nested/main.mjs';
import {readFileSync} from 'node:fs';
console.log(image === duplicate, same(), (await dynamic()).default === image,
    [...readFileSync(new URL(image, import.meta.url))].join(','));
"#
        ),
        b"true true true 0,255,128,10,13,42\n"
    );
    assert_eq!(cold.counters.source_reparses, 0);
    assert_eq!(cold.counters.fragment_reparses, 0);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.product.artifacts, cold.product.artifacts);
    let output = TmpDir::new();
    let cancel = crate::publish::Cancellation::default();
    let published =
        crate::publish::publish(output.root(), &cold.product.artifacts, &cancel).unwrap();
    assert_eq!(fs::read(published.directory.join(asset)).unwrap(), original);
    assert!(
        !crate::publish::publish(output.root(), &cold.product.artifacts, &cancel)
            .unwrap()
            .changed
    );
    fs::write(dir.root().join("image.png"), [254, 253, 0]).unwrap();
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 1);
    assert_eq!(edited.product.artifacts.resources.len(), 2);
    let next = crate::publish::publish(output.root(), &edited.product.artifacts, &cancel).unwrap();
    assert_ne!(next.directory, published.directory);
    assert_eq!(fs::read(published.directory.join(asset)).unwrap(), original);
    assert_eq!(
        crate::publish::prune_stale(output.root()).unwrap().removed,
        vec![published.directory]
    );
    for (path, bytes) in &edited.product.artifacts.resources {
        assert_eq!(&fs::read(next.directory.join(path)).unwrap(), bytes);
    }
    let mut unsafe_artifacts = edited.product.artifacts.clone();
    unsafe_artifacts
        .resources
        .insert("../escape".into(), vec![255]);
    assert!(crate::publish::publish(output.root(), &unsafe_artifacts, &cancel).is_err());
    unsafe_artifacts.resources.remove(Path::new("../escape"));
    unsafe_artifacts
        .resources
        .insert(unsafe_artifacts.entries["main"].clone(), vec![255]);
    assert!(crate::publish::publish(output.root(), &unsafe_artifacts, &cancel).is_err());
}

#[test]
fn binary_asset_entries_export_urls_and_dead_dynamic_assets_are_not_emitted() {
    let dir = TmpDir::new();
    fs::write(dir.root().join("data.wasm"), [0, 97, 115, 109, 255]).unwrap();
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "data.wasm".into();
    let entry = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        entry.product.artifacts.entries["main"],
        PathBuf::from("data.cjs")
    );
    let asset = entry.product.artifacts.resources.keys().next().unwrap();
    assert_eq!(
        observe_named(
            &entry.product.artifacts,
            "import url from './data.cjs'; console.log(url);"
        ),
        format!("./{}\n", asset.display()).as_bytes()
    );
    write(
        dir.root(),
        "main.mjs",
        "function dead() { return import('./data.wasm'); } console.log('main');",
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert!(built.product.artifacts.resources.is_empty());
    assert_eq!(observe(&built.product.artifacts), b"main\n");
}

#[test]
fn json_modules_share_values_across_import_require_dynamic_and_warm_builds() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import data from './data.json';
import load from './loader.cjs';
const dynamic = await import('./data.json');
data.changed = 3;
console.log(data === load(), data === dynamic.default, load().changed, data.value);
"#,
    );
    write(
        dir.root(),
        "loader.cjs",
        "module.exports = () => require('./data.json');",
    );
    write(dir.root(), "data.json", r#"{"value": 7}"#);
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(observe(&cold.product.artifacts), b"true true 3 7\n");
    assert_eq!(cold.counters.source_reparses, 0);
    assert_eq!(cold.counters.fragment_reparses, 0);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.counters.disk_hits, 3);
    assert_eq!(cold.product.artifacts, warm.product.artifacts);
    write(dir.root(), "data.json", r#"{"value": 9}"#);
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 1);
    assert!(edited
        .invalidated
        .contains(&dir.root().join("main.mjs").canonicalize().unwrap()));
    assert_eq!(observe(&edited.product.artifacts), b"true true 3 9\n");
    let output = TmpDir::new();
    let published = crate::publish::publish(
        output.root(),
        &edited.product.artifacts,
        &crate::publish::Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        observe_entry(&published.directory.join(&published.entries["main"])),
        b"true true 3 9\n"
    );
    write(dir.root(), "data.json", "{ broken json }");
    let error = session.build(&config).err().unwrap();
    assert!(format!("{error:#}").contains("data.json: invalid JSON module"));
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &edited.product));
}

#[test]
fn json_modules_preserve_literal_semantics_without_mutable_host_parsers() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import read from './loader.cjs';
JSON.parse = () => { throw Error('must not parse at runtime'); };
const data = read();
console.log(Object.hasOwn(data, '__proto__'), data.__proto__.own,
    Object.getPrototypeOf(data) === Object.prototype, data.duplicate,
    Object.is(data.zero, -0), data.large, data.surrogate.charCodeAt(0), data.escaped);
"#,
    );
    write(
        dir.root(),
        "loader.cjs",
        "module.exports = () => require('./data.json');",
    );
    write(
        dir.root(),
        "data.json",
        concat!(
            "\u{feff}",
            r#"{
"__proto__": {"own": true}, "duplicate": 1, "duplicate": 2,
"zero": -0, "large": 1e400, "surrogate": "\ud800", "escaped": "quote\"slash\\"
}"#
        ),
    );
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe(&built.product.artifacts),
        b"true true true 2 true Infinity 55296 quote\"slash\\\n"
    );
}

#[test]
fn json_entry_exports_values_and_uses_a_javascript_output_extension() {
    let dir = TmpDir::new();
    for json in ["null", "false", "42", r#""hello""#, "[1,2]"] {
        write(dir.root(), "data.json", json);
        let mut config = options();
        config.entries.get_mut("main").unwrap().path = "data.json".into();
        let built = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(
            built.product.artifacts.entries["main"],
            PathBuf::from("data.cjs")
        );
        assert_eq!(
            observe_named(
                &built.product.artifacts,
                "import value from './data.cjs'; console.log(JSON.stringify(value));"
            ),
            format!("{json}\n").as_bytes()
        );
    }
}

#[test]
fn native_externals_preserve_imports_reexports_dynamic_identity_and_entry_renaming() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import path, {basename} from 'node:path';
import * as ns from 'node:path';
export {basename as base} from 'node:path';
export * as pathNS from 'node:path';
export const load = () => import('node:path');
console.log(basename('/a/b'), path === ns.default, ns === await load());
"#,
    );
    let mut config = options();
    config.externals.insert("node:path".into());
    config.entries.get_mut("main").unwrap().output_name = Some("nested/start.mjs".into());
    let first = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe(&first.product.artifacts), b"b true true\n");
    assert_eq!(observe_named(&first.product.artifacts,
        "import {base,pathNS,load} from './nested/start.mjs'; console.log(base('/x/y'), pathNS === await load());"),
        b"b true true\ny true\n");
    assert!(first.product.artifacts.async_facades.is_empty());
    assert_eq!(first.product.artifacts.manifest.async_facades.len(), 1);
    assert!(first
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("\"node:path\"")));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(first.product.artifacts, warm.product.artifacts);
}

#[test]
fn native_external_star_surfaces_are_owned_and_validated_by_the_native_loader() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs",
        "export * from './barrel.mjs'; import {basename} from './barrel.mjs'; console.log(basename('/a/b'));");
    write(
        dir.root(),
        "barrel.mjs",
        "export * from 'node:fs'; export * from 'node:path';",
    );
    let mut config = options();
    config
        .externals
        .extend(["node:fs".into(), "node:path".into()]);
    let first = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe_named(&first.product.artifacts,
        "import * as ns from './main.mjs'; console.log(typeof ns.basename, typeof ns.readFileSync, 'default' in ns);"),
        b"b\nfunction function false\n");
    write(
        dir.root(),
        "main.mjs",
        "import {doesNotExist} from 'node:path'; console.log(doesNotExist);",
    );
    let missing = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe_named(
            &missing.product.artifacts,
            "await import('./main.mjs').catch(e => console.log(e.name));"
        ),
        b"SyntaxError\n"
    );
}

#[test]
fn native_external_evaluation_order_and_global_mutations_remain_observable() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import 'external-order'; import './local.mjs'; Math.abs(1); console.log('main');",
    );
    write(dir.root(), "local.mjs", "console.log('local');");
    let mut config = options();
    config.externals.insert("external-order".into());
    let first = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let mut artifacts = first.product.artifacts.clone();
    artifacts.files.insert(
        "node_modules/external-order/index.js".into(),
        "console.log('external'); Math.abs = () => console.log('patched');".into(),
    );
    assert_eq!(observe(&artifacts), b"external\nlocal\npatched\nmain\n");
}

#[test]
fn native_external_requires_and_imports_remain_lazy_and_use_host_caches() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './loader.cjs';
console.log('start');
const first = api.load();
console.log(first.value, first === api.load(), first === (await api.dynamic()).default);
try { await api.missing(); } catch (e) { console.log(e.code); }
"#,
    );
    write(dir.root(), "loader.cjs",
        "exports.load = () => require('runtime-lazy'); exports.dynamic = () => import('runtime-lazy'); exports.missing = () => import('not-installed');");
    let mut config = options();
    config
        .externals
        .extend(["runtime-lazy".into(), "not-installed".into()]);
    let first = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let mut artifacts = first.product.artifacts.clone();
    artifacts.files.insert(
        "node_modules/runtime-lazy/index.js".into(),
        "console.log('loaded'); exports.value = 9;".into(),
    );
    assert_eq!(
        observe(&artifacts),
        b"start\nloaded\n9 true true\nERR_MODULE_NOT_FOUND\n"
    );
}

#[test]
fn native_external_unused_imports_and_reexports_still_validate_names() {
    for (main, bad) in [
        (
            "import {doesNotExist} from 'node:path'; console.log('ran');",
            "",
        ),
        (
            "import './bad.mjs'; console.log('ran');",
            "export {doesNotExist} from 'node:path';",
        ),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", main);
        write(dir.root(), "bad.mjs", bad);
        let mut config = options();
        config.externals.insert("node:path".into());
        let product = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(
            observe_named(
                &product.product.artifacts,
                "await import('./main.mjs').catch(e => console.log(e.name));"
            ),
            b"SyntaxError\n"
        );
    }
}

#[test]
fn native_external_star_collisions_preserve_cjs_candidates_without_explicit_override() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {value} from './barrel.mjs'; console.log(value);",
    );
    write(
        dir.root(),
        "barrel.mjs",
        "export * from './dep.cjs'; export * from 'node:path';",
    );
    write(dir.root(), "dep.cjs", "exports.value = 1;");
    let mut config = options();
    config.externals.insert("node:path".into());
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe(&product.product.artifacts), b"1\n");
    write(
        dir.root(),
        "main.mjs",
        "import {value} from './dep.cjs'; import 'node:path'; console.log('unused');",
    );
    let unused = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe(&unused.product.artifacts), b"unused\n");
    write(
        dir.root(),
        "main.mjs",
        "import {basename} from './barrel.mjs'; console.log(basename);",
    );
    write(dir.root(), "dep.cjs", "exports.basename = 'own';");
    let collision = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe_named(
            &collision.product.artifacts,
            "await import('./main.mjs').catch(e => console.log(e.name));"
        ),
        b"SyntaxError\n"
    );
}

#[test]
fn relative_context_imports_preserve_lazy_namespaces_and_warm_candidate_changes() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
export const load = name => import(`./parts/${name}`);
console.log('start');
const a = await load('a.mjs');
console.log(a.value, a === await load('a.mjs'));
const b = await load('b.cjs');
console.log(b.value, b === await load('b.cjs'));
try { const promise = load('missing.mjs'); console.log('promise', promise instanceof Promise); await promise; } catch (e) { console.log(e.code); }
"#,
    );
    write(
        dir.root(),
        "parts/a.mjs",
        "console.log('a'); export const value = 1;",
    );
    write(
        dir.root(),
        "parts/b.cjs",
        "console.log('b'); exports.value = 2;",
    );
    write(
        dir.root(),
        "parts/unused.mjs",
        "console.log('unused'); export const value = 3;",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&options()).unwrap();
    assert_eq!(
        observe(&first.product.artifacts),
        b"start\na\n1 true\nb\n2 true\npromise true\nERR_MODULE_NOT_FOUND\n"
    );
    assert_eq!(first.product.artifacts.async_facades.len(), 3);
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.product.artifacts, first.product.artifacts);
    write(dir.root(), "parts/added.mjs", "export const value = 4;");
    let added = session.build(&options()).unwrap();
    assert_eq!(added.counters.source_parses, 1);
    assert_eq!(added.product.artifacts.async_facades.len(), 4);
    fs::remove_file(dir.root().join("parts/unused.mjs")).unwrap();
    let removed = session.build(&options()).unwrap();
    assert_eq!(removed.product.artifacts.async_facades.len(), 3);
    assert_eq!(
        observe(&removed.product.artifacts),
        observe(&first.product.artifacts)
    );
}

#[test]
fn relative_context_imports_keep_argument_rewrites_awaits_and_rejections() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import {name} from './name.mjs';
const load = () => import('./parts/' + name + '.mjs');
const nested = await import(`./parts/${(await import('./name.mjs')).name}.mjs`);
console.log(nested.value, nested === await load());
const bad = {toString() { throw new Error('argument'); }};
try {
  const promise = import(`./parts/${bad}.mjs`);
  console.log('promise', promise instanceof Promise);
  await promise;
} catch (error) { console.log(error.message); }
try { await import(`./empty/${name}.mjs`); } catch (error) { console.log(error.code); }
"#,
    );
    write(dir.root(), "name.mjs", "export const name = 'a';");
    write(dir.root(), "parts/a.mjs", "export const value = 5;");
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe(&product.product.artifacts),
        b"5 true\nargument\nERR_MODULE_NOT_FOUND\n"
    );
    assert_eq!(product.counters.source_reparses, 0);
    assert_eq!(product.counters.fragment_reparses, 0);
}

#[test]
fn relative_context_requires_capture_extensionless_and_index_aliases_without_eager_execution() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './main.cjs';");
    write(
        dir.root(),
        "main.cjs",
        r#"
const load = name => require('./parts/' + name);
console.log('start');
console.log(load('a').value, load('a') === load('a.js'));
console.log(load('nested').value, load('nested') === load('nested/'));
try { load('missing'); } catch (e) { console.log(e.code); }
module.exports = load;
"#,
    );
    write(
        dir.root(),
        "parts/a.js",
        "console.log('a'); exports.value = 1;",
    );
    write(
        dir.root(),
        "parts/nested/index.js",
        "console.log('nested'); exports.value = 2;",
    );
    write(dir.root(), "parts/unused.js", "console.log('unused');");
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe(&product.product.artifacts),
        b"start\na\n1 true\nnested\n2 true\nMODULE_NOT_FOUND\n"
    );
}

#[test]
fn relative_contexts_do_not_activate_dead_sites_and_compose_nested_commonjs_dependencies() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
function dead(name) { return import(`./dead/${name}.mjs`); }
export const kept = 1;
import './main.cjs';
"#,
    );
    write(dir.root(), "dead/unused.mjs", "console.log('dead');");
    write(dir.root(), "name.cjs", "module.exports = 'a';");
    write(
        dir.root(),
        "main.cjs",
        r#"
console.log(require(`./parts/${require('./name.cjs')}.js`).value);
module.exports = import('./parts/' + 'a.js').then(ns => console.log(ns.value));
"#,
    );
    write(
        dir.root(),
        "parts/a.js",
        "console.log('a'); exports.value = 7;",
    );
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&product.product.artifacts), b"a\n7\n7\n");
    assert_eq!(product.product.artifacts.async_facades.len(), 1);
    assert!(product
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("console.log('dead')")));
}

#[test]
fn relative_context_requires_reject_every_async_esm_candidate() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './main.cjs';");
    write(
        dir.root(),
        "main.cjs",
        "module.exports = name => require(`./parts/${name}.mjs`);",
    );
    write(dir.root(), "parts/sync.mjs", "export const value = 1;");
    write(
        dir.root(),
        "parts/async.mjs",
        "await Promise.resolve(); export const value = 2;",
    );
    let error = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("require of async ESM"),
        "{error:?}"
    );
}

#[test]
fn relative_contexts_separate_url_keys_from_paths_and_avoid_error_binding_capture() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
export const load = (Error, name) => import(`./parts/${name}.mjs`);
for (const name of ['a b', 'a%20b', 'caf\u00e9', 'caf%C3%A9', 'a%25%23b']) {
  console.log((await load(0, name)).value);
}
try {
  const promise = load(0, 'absent');
  console.log('promise', promise instanceof Promise);
  await promise;
} catch (e) { console.log(e.name, e.code); }
"#,
    );
    write(dir.root(), "parts/a b.mjs", "export const value = 'space';");
    write(
        dir.root(),
        "parts/caf\u{e9}.mjs",
        "export const value = 'unicode';",
    );
    write(
        dir.root(),
        "parts/a%#b.mjs",
        "export const value = 'reserved';",
    );
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe(&product.product.artifacts),
        b"space\nspace\nunicode\nunicode\nreserved\npromise true\nError ERR_MODULE_NOT_FOUND\n"
    );
    assert_eq!(product.product.artifacts.async_facades.len(), 3);
}

#[test]
fn connected_effects_specialize_imported_functions_through_barrels() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {read} from './barrel.mjs'; read({value: 1}); export const kept = 1;",
    );
    write(dir.root(), "barrel.mjs", "export {read} from './read.mjs';");
    write(
        dir.root(),
        "read.mjs",
        "export function read(x) { return x.value; }",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&options()).unwrap();
    assert_eq!(observe(&first.product.artifacts), b"");
    assert!(first
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("function read")));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.counters.disk_hits, 3);
    assert_eq!(warm.product.artifacts, first.product.artifacts);
    let noop = session.build(&options()).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    write(
        dir.root(),
        "read.mjs",
        "export function read(x) { console.log('read'); return x.value; }",
    );
    let changed = session.build(&options()).unwrap();
    assert_eq!(changed.counters.source_parses, 1);
    assert_eq!(observe(&changed.product.artifacts), b"read\n");
}

#[test]
fn opaque_commonjs_mutations_invalidate_environment_purity() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './patch.cjs'; Math.abs(1);");
    write(
        dir.root(),
        "patch.cjs",
        "Math.abs = function () { console.log('patched'); return 0; };",
    );
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&product.product.artifacts), b"patched\n");
}

#[test]
fn connected_effects_preserve_deferred_tdz_observations() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "function read() { return value; } read(); let value = 1;",
    );
    let product = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe_named(
            &product.product.artifacts,
            "await import('./main.mjs').catch(error => console.log(error.name));"
        ),
        b"ReferenceError\n"
    );
}

#[test]
fn declaration_changes_invalidate_memory_disk_and_build_products() {
    use ast_segmenter::fingerprint::declaration_fingerprint_count;

    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './dep.mjs'; Math.abs(-1); export const kept = 1;",
    );
    write(dir.root(), "dep.mjs", "export const value = 1;");
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let hashes = declaration_fingerprint_count();
    let first = session.build(&config).unwrap();
    assert_eq!(first.counters.source_parses, 2);
    assert_eq!(declaration_fingerprint_count(), hashes + 1);
    let noop = session.build(&config).unwrap();
    assert_eq!(noop.counters.memory_hits, 2);
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert_eq!(declaration_fingerprint_count(), hashes + 1);
    assert!(first
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("Math.abs")));
    config.declarations.user.push(
        ecco::parse_declarations("global fn Math.abs(value) { call_effects = true }").unwrap(),
    );
    let changed = session.build(&config).unwrap();
    assert_eq!(changed.counters.source_parses, 2);
    assert_eq!(declaration_fingerprint_count(), hashes + 2);
    assert!(changed
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("Math.abs")));
    assert_ne!(
        first.product.fingerprints.parse,
        changed.product.fingerprints.parse
    );
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(warm.counters.disk_hits, 2);
    assert_eq!(declaration_fingerprint_count(), hashes + 3);
    assert_eq!(warm.product.artifacts, changed.product.artifacts);
    config.declarations.user.clear();
    let restored = session.build(&config).unwrap();
    assert_eq!(restored.counters.source_parses, 2);
    assert_eq!(declaration_fingerprint_count(), hashes + 4);
    assert_eq!(restored.product.artifacts, first.product.artifacts);

    config.declarations.user.push(
        ecco::parse_declarations("global fn invalid() { call_effects = false }").unwrap(),
    );
    config
        .declarations
        .user
        .push(config.declarations.user[0].clone());
    assert!(session.build(&config).is_err());
    assert_eq!(declaration_fingerprint_count(), hashes + 4);
    config.declarations.user.clear();
    let recovered = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&restored.product, &recovered.product));
    assert_eq!(declaration_fingerprint_count(), hashes + 4);
}

#[test]
fn effect_cycle_warnings_survive_noop_and_clear_after_source_change() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './dep.mjs'; console.log('main');",
    );
    write(
        dir.root(),
        "dep.mjs",
        "import './main.mjs'; console.log('dep');",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&options()).unwrap();
    assert_eq!(first.warnings.len(), 1);
    assert!(first.warnings[0].contains("effect-order cycle"));
    let noop = session.build(&options()).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert_eq!(first.warnings, noop.warnings);
    assert_eq!(noop.counters.emissions, 0);
    write(dir.root(), "dep.mjs", "console.log('dep');");
    let changed = session.build(&options()).unwrap();
    assert!(changed.warnings.is_empty());
    assert!(!Rc::ptr_eq(&first.product, &changed.product));
}

fn parity(session: &mut BuildSession, options: &BuildOptions) -> BuildResult {
    let incremental = session.build(options).unwrap();
    let mut full_options = options.clone();
    full_options.cache = CacheMode::Disabled;
    let full = BuildSession::new(session.root())
        .unwrap()
        .build(&full_options)
        .unwrap();
    assert_eq!(incremental.product.fingerprints, full.product.fingerprints);
    assert_eq!(incremental.product.artifacts, full.product.artifacts);
    assert_eq!(
        incremental.product.plan.node_roots,
        full.product.plan.node_roots
    );
    assert_eq!(
        format!("{:?}", incremental.product.graph.edges()),
        format!("{:?}", full.product.graph.edges())
    );
    assert_eq!(
        observe(&incremental.product.artifacts),
        observe(&full.product.artifacts)
    );
    let graph = &incremental.product.graph;
    let sources = graph
        .module_ids()
        .map(|file| {
            let path = graph.file_path(file).unwrap().to_owned();
            let source = session.sources[&path].text.clone();
            (path, source)
        })
        .collect();
    let mut checked =
        link::esm_plan(graph, &incremental.product.plan, &options.effects, &sources).unwrap();
    let destinations = entry_outputs::apply(session.root(), options, &mut checked).unwrap();
    let mut checked = checked.render_to(&destinations).unwrap();
    // Low-level emission consumes prepared JS, not original resource discovery.
    checked.resources = incremental.product.artifacts.resources.clone();
    checked.stylesheets = incremental.product.artifacts.stylesheets.clone();
    assert_eq!(incremental.product.artifacts, checked);
    incremental
}

fn observe(artifacts: &Artifacts) -> Vec<u8> {
    let dir = TmpDir::new();
    for (path, bytes) in artifacts.file_bytes() {
        let path = dir.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    observe_entry(&dir.root().join(&artifacts.entries["main"]))
}

fn observe_entry(entry: &Path) -> Vec<u8> {
    let output = std::process::Command::new("node")
        .args(["-e", "const {spawnSync}=require('node:child_process'); const r=spawnSync(process.execPath,[process.argv[1]],{timeout:5000,killSignal:'SIGKILL'}); if(r.error||r.status!==0||r.stderr.length) {console.error(r.error||r.stderr.toString());process.exit(1)} process.stdout.write(r.stdout)"])
        .arg(entry)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn observe_named(artifacts: &Artifacts, check: &str) -> Vec<u8> {
    let dir = TmpDir::new();
    for path in artifacts
        .entries
        .values()
        .chain(artifacts.modules.values())
        .chain(artifacts.members.values())
        .chain(artifacts.chunk_artifacts.values().flatten())
        .chain(artifacts.cjs_proxies.values().map(|(path, _)| path))
        .chain(artifacts.cjs_namespaces.values())
        .chain(artifacts.async_facades.values())
    {
        assert!(
            artifacts.files.contains_key(path),
            "missing artifact {}",
            path.display()
        );
    }
    for (path, bytes) in artifacts.file_bytes() {
        let path = dir.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    write(dir.root(), "check.mjs", check);
    observe_entry(&dir.root().join("check.mjs"))
}

#[test]
fn named_entry_outputs_preserve_single_file_default_live_bindings_and_cache() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.mjs",
        "export let value = 1; export {value as default}; export function bump() { value++; }",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    for (output_name, expected) in [
        (None, "entry.mjs"),
        (Some("./entry.mjs"), "entry.mjs"),
        (Some("appstart"), "appstart.mjs"),
        (Some("nested/start"), "nested/start.mjs"),
        (Some("nested/start.mjs"), "nested/start.mjs"),
        (Some("nested/#100%.mjs"), "nested/#100%.mjs"),
        (Some("nested/start.js"), "nested/start.js"),
    ] {
        let options = BuildOptions {
            entries: BTreeMap::from([(
                "startup".into(),
                Entry {
                    path: "./entry.mjs".into(),
                    output_name: output_name.map(PathBuf::from),
                    namespace: true,
                },
            )]),
            ..Default::default()
        };
        let build = session.build(&options).unwrap();
        let artifacts = &build.product.artifacts;
        let path = Path::new(expected);
        assert_eq!(artifacts.entries["startup"], path);
        assert!(artifacts.files.contains_key(path));
        assert_eq!(
            artifacts.files.len(),
            if expected.ends_with(".js") { 2 } else { 1 }
        );
        assert!(artifacts.modules.values().all(|p| p == path));
        assert!(artifacts.members.values().all(|p| p == path));
        assert!(artifacts
            .chunk_artifacts
            .values()
            .flatten()
            .all(|p| p == path));
        let request = expected.replace('%', "%25").replace('#', "%23");
        assert_eq!(
            observe_named(
                artifacts,
                &format!(
                    "import * as ns from './{request}'; ns.bump(); console.log(ns.default, ns.value);"
                )
            ),
            b"2 2\n"
        );
        let noop = session.build(&options).unwrap();
        assert!(Rc::ptr_eq(&build.product, &noop.product));
        assert_eq!(noop.counters.output_parses, 0);
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&options)
            .unwrap();
        assert_eq!(warm.product.artifacts, *artifacts);
    }
}

#[test]
fn default_output_names_preserve_nested_sources_and_share_logical_aliases() {
    let dir = TmpDir::new();
    write(dir.root(), "src/entry.mjs", "export const value = 1;");
    let mut options = BuildOptions {
        entries: BTreeMap::from([
            (
                "../logical-id".into(),
                Entry {
                    path: "./src/entry.mjs".into(),
                    output_name: None,
                    namespace: true,
                },
            ),
            (
                "other-id".into(),
                Entry {
                    path: dir.root().join("src/entry.mjs"),
                    output_name: None,
                    namespace: true,
                },
            ),
            (
                "explicit-id".into(),
                Entry {
                    path: "src/entry.mjs".into(),
                    output_name: Some("src/entry".into()),
                    namespace: true,
                },
            ),
        ]),
        ..Default::default()
    };
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    assert!(build
        .product
        .artifacts
        .entries
        .values()
        .all(|path| path == Path::new("src/entry.mjs")));
    assert_eq!(build.product.artifacts.files.len(), 1);
    assert_eq!(
        observe_named(
            &build.product.artifacts,
            "import {value} from './src/entry.mjs'; console.log(value);"
        ),
        b"1\n"
    );
    options
        .entries
        .get_mut("../logical-id")
        .unwrap()
        .output_name = Some("first.mjs".into());
    let aliased = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    assert_eq!(
        aliased.product.artifacts.entries["../logical-id"],
        Path::new("first.mjs")
    );
    assert_eq!(
        aliased.product.artifacts.entries["other-id"],
        Path::new("src/entry.mjs")
    );
    assert_eq!(
        aliased.product.artifacts.entries["explicit-id"],
        Path::new("src/entry.mjs")
    );
    assert_eq!(aliased.product.artifacts.files.len(), 2);
    assert_eq!(
        observe_named(
            &aliased.product.artifacts,
            "import {value} from './src/entry.mjs'; console.log(value);"
        ),
        b"1\n"
    );
}

#[test]
fn named_entry_outputs_rewrite_static_reexports_and_dynamic_canonical_namespaces() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.mjs",
        "import * as ns from './dep.mjs'; export {ns}; export {value} from './dep.mjs'; export function load() { return import('./dep.mjs'); }",
    );
    write(
        dir.root(),
        "dep.mjs",
        "await Promise.resolve(); export let value = 1; export {value as default}; export function bump() { value++; }",
    );
    let options = BuildOptions {
        entries: BTreeMap::from([
            (
                "app/main.mjs".into(),
                Entry {
                    path: "entry.mjs".into(),
                    output_name: Some("app/main.mjs".into()),
                    namespace: true,
                },
            ),
            (
                "lib/#dep%.mjs".into(),
                Entry {
                    path: "dep.mjs".into(),
                    output_name: Some("lib/#dep%.mjs".into()),
                    namespace: true,
                },
            ),
        ]),
        ..Default::default()
    };
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    let artifacts = &build.product.artifacts;
    assert_eq!(artifacts.files.len(), 2);
    assert_eq!(
        artifacts.modules[&dir.root().join("dep.mjs")],
        Path::new("lib/#dep%.mjs")
    );
    assert!(artifacts
        .async_facades
        .values()
        .all(|p| p == Path::new("lib/#dep%.mjs")));
    assert_eq!(
        observe_named(
            artifacts,
            r#"
import * as entry from './app/main.mjs';
import * as dep from './lib/%23dep%25.mjs';
console.log(entry.ns === dep, await entry.load() === dep);
dep.bump(); console.log(entry.value, dep.default);
"#
        ),
        b"true true\n2 2\n"
    );
}

#[test]
fn named_entry_outputs_keep_cycle_tdz_and_do_not_rewrite_shadowed_require_or_strings() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.mjs",
        "export const label = './native.mjs'; export function fake(require) { return require('./missing.mjs'); }",
    );
    let options = BuildOptions {
        entries: BTreeMap::from([(
            "app/entry.mjs".into(),
            Entry {
                path: "entry.mjs".into(),
                output_name: Some("app/entry.mjs".into()),
                namespace: true,
            },
        )]),
        ..Default::default()
    };
    let mut session = BuildSession::new(dir.root()).unwrap();
    let strings = session.build(&options).unwrap();
    assert_eq!(
        observe_named(
            &strings.product.artifacts,
            "import {label, fake} from './app/entry.mjs'; console.log(label, fake(x => x));"
        ),
        b"./native.mjs ./missing.mjs\n"
    );
    write(
        dir.root(),
        "entry.mjs",
        "import {b} from './dep.mjs'; export const a = b;",
    );
    write(
        dir.root(),
        "dep.mjs",
        "import {a} from './entry.mjs'; export const b = a;",
    );
    let cycle = session.build(&options).unwrap();
    assert_eq!(
        observe_named(
            &cycle.product.artifacts,
            "await import('./app/entry.mjs').catch(error => console.log(error.name));"
        ),
        b"ReferenceError\n"
    );
}

#[test]
fn named_entry_outputs_aliases_forward_default_and_live_exports() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.mjs",
        "globalThis.runs = (globalThis.runs || 0) + 1; export let value = 1; export {value as default}; export function bump() { value++; } export function load() { return import('./entry.mjs'); }",
    );
    let options = BuildOptions {
        entries: ["a.mjs", "nested/z.mjs"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Entry {
                        path: "entry.mjs".into(),
                        output_name: Some(name.into()),
                        namespace: true,
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    let artifacts = &build.product.artifacts;
    assert_eq!(artifacts.files.len(), 2);
    assert_eq!(artifacts.entries["a.mjs"], Path::new("a.mjs"));
    assert_eq!(artifacts.entries["nested/z.mjs"], Path::new("nested/z.mjs"));
    assert!(artifacts
        .async_facades
        .values()
        .all(|p| p == Path::new("a.mjs")));
    assert_eq!(
        observe_named(
            artifacts,
            r#"
import * as canonical from './a.mjs';
import * as alias from './nested/z.mjs';
alias.bump(); console.log(alias.default, canonical.value, globalThis.runs);
console.log(await alias.load() === canonical, alias.bump === canonical.bump);
"#
        ),
        b"2 2 1\ntrue true\n"
    );
}

#[test]
fn alias_destination_can_reuse_the_original_artifact_path() {
    let dir = TmpDir::new();
    write(dir.root(), "entry.mjs",
        "globalThis.runs = (globalThis.runs || 0) + 1; export default 7; export let value = 1; export function bump(){ value++; }");
    let options = BuildOptions {
        entries: [("a", "app.mjs"), ("z", "native.mjs")]
            .into_iter()
            .map(|(id, name)| {
                (
                    id.into(),
                    Entry {
                        path: "entry.mjs".into(),
                        output_name: Some(name.into()),
                        namespace: true,
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    assert_eq!(build.product.artifacts.files.len(), 2);
    assert_eq!(observe_named(&build.product.artifacts,
        "import * as first from './app.mjs'; import * as alias from './native.mjs'; alias.bump(); console.log(alias.default, first.value, globalThis.runs);"),
        b"7 2 1\n");
}

#[test]
fn named_entry_outputs_commonjs_aliases_keep_cache_getters_and_reassigned_exports() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.cjs",
        "globalThis.runs = (globalThis.runs || 0) + 1; Object.defineProperty(exports, 'value', {get: () => 7}); exports.replace = () => { module.exports = {value: 9}; };",
    );
    for name in [
        "entry.cjs",
        "appstart",
        "nested/start.cjs",
        "nested/start.js",
    ] {
        let options = BuildOptions {
            entries: [name, "z-alias.cjs"]
                .into_iter()
                .map(|key| {
                    (
                        key.into(),
                        Entry {
                            path: "entry.cjs".into(),
                            output_name: Some(key.into()),
                            namespace: true,
                        },
                    )
                })
                .collect(),
            ..Default::default()
        };
        let build = BuildSession::new(dir.root())
            .unwrap()
            .build(&options)
            .unwrap();
        let artifacts = &build.product.artifacts;
        let path = if name == "appstart" {
            "appstart.cjs"
        } else {
            name
        };
        assert_eq!(artifacts.entries[name], Path::new(path));
        assert_eq!(
            observe_named(
                artifacts,
                &format!(
                    r#"
import {{createRequire}} from 'node:module';
const require = createRequire(import.meta.url);
const first = require('./{path}');
const alias = require('./z-alias.cjs');
console.log(first === alias, first.value, globalThis.runs, !!Object.getOwnPropertyDescriptor(alias, 'value').get);
first.replace(); console.log(require('./{path}').value, require('./z-alias.cjs').value, globalThis.runs);
"#
                )
            ),
            b"true 7 1 true\n9 9 1\n"
        );
    }
}

#[test]
fn named_entry_outputs_rewrite_deferred_require_and_import_from_commonjs() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.cjs",
        "exports.load = () => require('./dep.mjs'); exports.dynamic = () => import('./dep.mjs');",
    );
    write(
        dir.root(),
        "dep.mjs",
        "globalThis.runs = (globalThis.runs || 0) + 1; export const value = 17;",
    );
    let options = BuildOptions {
        entries: BTreeMap::from([
            (
                "app/start".into(),
                Entry {
                    path: "entry.cjs".into(),
                    output_name: Some("app/start".into()),
                    namespace: true,
                },
            ),
            (
                "lib/#dep%.mjs".into(),
                Entry {
                    path: "dep.mjs".into(),
                    output_name: Some("lib/#dep%.mjs".into()),
                    namespace: true,
                },
            ),
        ]),
        ..Default::default()
    };
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    assert_eq!(
        build.product.artifacts.entries["app/start"],
        Path::new("app/start.cjs")
    );
    assert_eq!(
        observe_named(
            &build.product.artifacts,
            r#"
import {createRequire} from 'node:module';
import * as dep from './lib/%23dep%25.mjs';
const require = createRequire(import.meta.url);
const entry = require('./app/start.cjs');
console.log(entry.load().value, await entry.dynamic() === dep, globalThis.runs);
"#
        ),
        b"17 true 1\n"
    );
}

#[test]
fn named_entry_outputs_infer_js_input_extensions_and_module_format() {
    for format in ["module", "commonjs"] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "package.json",
            &format!(r#"{{"type":"{format}"}}"#),
        );
        write(
            dir.root(),
            "entry.js",
            if format == "module" {
                "export const value = 13;"
            } else {
                "exports.value = 13;"
            },
        );
        let options = BuildOptions {
            entries: BTreeMap::from([(
                "nested/start".into(),
                Entry {
                    path: "entry.js".into(),
                    output_name: Some("nested/start".into()),
                    namespace: true,
                },
            )]),
            ..Default::default()
        };
        let build = BuildSession::new(dir.root())
            .unwrap()
            .build(&options)
            .unwrap();
        assert_eq!(
            build.product.artifacts.entries["nested/start"],
            Path::new("nested/start.js")
        );
        assert_eq!(
            observe_named(
                &build.product.artifacts,
                &format!(
                    "import * as ns from './nested/start.js'; console.log({});",
                    if format == "module" {
                        "ns.value"
                    } else {
                        "ns.default.value"
                    }
                )
            ),
            b"13\n"
        );
    }
}

#[test]
fn named_entry_outputs_publication_uses_requested_relative_paths() {
    let dir = TmpDir::new();
    write(dir.root(), "entry.mjs", "export default 42;");
    let options = BuildOptions {
        entries: BTreeMap::from([(
            "app/start".into(),
            Entry {
                path: "entry.mjs".into(),
                output_name: Some("app/start".into()),
                namespace: true,
            },
        )]),
        ..Default::default()
    };
    let mut session = BuildSession::new(dir.root()).unwrap();
    let output = dir.root().join(".bundler-output");
    let cancel = crate::publish::Cancellation::default();
    let (build, published) = session
        .build_and_publish(&options, &output, &cancel)
        .unwrap();
    assert_eq!(
        build.product.artifacts.entries["app/start"],
        Path::new("app/start.mjs")
    );
    assert!(published.entries["app/start"].ends_with("app/start.mjs"));
    assert!(published.directory.join("app/start.mjs").is_file());
    write(
        dir.root(),
        ".bundler-check/check.mjs",
        &format!(
            "import value from {}; console.log(value);",
            serde_json::to_string(&published.entries["app/start"]).unwrap()
        ),
    );
    assert_eq!(
        observe_entry(&dir.root().join(".bundler-check/check.mjs")),
        b"42\n"
    );
    let (noop, unchanged) = session
        .build_and_publish(&options, &output, &cancel)
        .unwrap();
    assert!(!unchanged.changed);
    assert_eq!(unchanged.directory, published.directory);
    assert!(Rc::ptr_eq(&build.product, &noop.product));
}

#[test]
fn named_entry_outputs_reject_unsafe_reserved_incompatible_and_colliding_paths() {
    let dir = TmpDir::new();
    write(dir.root(), "entry.mjs", "export const value = 1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    for name in [
        "",
        "/entry.mjs",
        "../entry.mjs",
        "nested/../../entry.mjs",
        ".bundler-lock",
        "nested/.bundler-current.json",
        "C:\\entry.mjs",
        "entry.cjs",
        "entry.json",
        "nested/",
        "CON.mjs",
    ] {
        let options = BuildOptions {
            entries: BTreeMap::from([(
                name.into(),
                Entry {
                    path: "entry.mjs".into(),
                    output_name: Some(name.into()),
                    namespace: true,
                },
            )]),
            ..Default::default()
        };
        assert!(session.build(&options).is_err(), "{name}");
    }
    for names in [
        ["entry.mjs", "ENTRY.mjs"],
        ["entry.mjs", "entry.mjs/child.mjs"],
    ] {
        let options = BuildOptions {
            entries: names
                .into_iter()
                .map(|name| {
                    (
                        name.into(),
                        Entry {
                            path: "entry.mjs".into(),
                            output_name: Some(name.into()),
                            namespace: true,
                        },
                    )
                })
                .collect(),
            ..Default::default()
        };
        assert!(session.build(&options).is_err(), "{names:?}");
    }
    write(
        dir.root(),
        "entry.mjs",
        "export function load() { return import('./dep.mjs'); }",
    );
    write(dir.root(), "dep.mjs", "export const value = 1;");
    let mut options = BuildOptions {
        entries: BTreeMap::from([(
            "entry.mjs".into(),
            Entry {
                path: "entry.mjs".into(),
                output_name: None,
                namespace: true,
            },
        )]),
        ..Default::default()
    };
    let good = session.build(&options).unwrap();
    let generated = good
        .product
        .artifacts
        .files
        .keys()
        .find(|path| **path != Path::new("entry.mjs"))
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    options.entries.get_mut("entry.mjs").unwrap().output_name = Some(generated.into());
    let error = session.build(&options).err().unwrap().to_string();
    assert!(
        error.contains("collides with generated artifact"),
        "{error}"
    );
    write(dir.root(), "entry.cjs", "exports.value = 1;");
    options.entries = BTreeMap::from([(
        "wrong.mjs".into(),
        Entry {
            path: "entry.cjs".into(),
            output_name: Some("wrong.mjs".into()),
            namespace: true,
        },
    )]);
    assert!(session.build(&options).is_err());
    options.entries = BTreeMap::from([
        (
            "a.js".into(),
            Entry {
                path: "entry.cjs".into(),
                output_name: Some("a.js".into()),
                namespace: true,
            },
        ),
        (
            "b.js".into(),
            Entry {
                path: "entry.mjs".into(),
                output_name: Some("b.js".into()),
                namespace: true,
            },
        ),
    ]);
    let error = session.build(&options).err().unwrap().to_string();
    assert!(error.contains("incompatible .js module formats"), "{error}");
}

#[test]
fn binding_demand_retains_all_redeclaration_initializers() {
    for (imported, dependency, expected) in [
        ("get", "var value = 1; var value = 2; export function get() { return value; }", "2\n"),
        ("get", "var value; var value = 1; var value; var value = 2; var value; export function get() { return value; }", "2\n"),
        ("value", "var value = 1; var value = 2; var value; export {value};", "2\n"),
        ("value", "export var value = 1; var value = 2; var value;", "2\n"),
        ("value", "var value = 1; export var value = 2; var value;", "2\n"),
        ("value", "export var value = 1; var value;", "1\n"),
        ("get", "var value = 1; var value = 2; function shadow() { let value = 3; return value; } export function get() { return [value, shadow()].join(','); }", "2,3\n"),
        ("get", "export function get() { var value; function value() { return 1; } var value = 2; var value; return value; }", "2\n"),
        ("get", "export function get() { var value; function value() { return 1; } var value; return value(); }", "1\n"),
    ] {
        let dir = TmpDir::new();
        let expression = if imported == "get" { "get()" } else { "value" };
        write(
            dir.root(),
            "main.mjs",
            &format!("import {{ {imported} }} from './dep.mjs'; console.log({expression});"),
        );
        write(dir.root(), "dep.mjs", &format!("{dependency} var unrelated = 99;"));
        assert_eq!(observe_entry(&dir.root().join("main.mjs")), expected.as_bytes());
        let build = parity(&mut BuildSession::new(dir.root()).unwrap(), &options());
        assert_eq!(observe(&build.product.artifacts), expected.as_bytes(), "{dependency}");
        assert!(build.product.artifacts.files.values().all(|text| !text.contains("unrelated")));
    }
}

#[test]
fn optional_pure_await_preserves_native_guards_and_microtask_timing() {
    for (invocation, expected) in [
        (
            "globalThis.run?.(await value())",
            "value,microtask,call,after",
        ),
        ("obj?.run(await value())", "get,value,microtask,call,after"),
        ("obj.run?.(await value())", "get,value,microtask,call,after"),
        (
            "obj?.run?.(await value())",
            "get,value,microtask,call,after",
        ),
        (
            "obj?.[await Promise.resolve(key())](await value())",
            "key,microtask,get,value,call,after",
        ),
        (
            "obj?.[await Promise.resolve(key())](1)",
            "key,microtask,get,call,after",
        ),
        (
            "(await Promise.resolve(receiver()))?.run(await value())",
            "receiver,microtask,get,value,call,after",
        ),
        (
            "obj?.run(await value()).run(1)",
            "get,value,microtask,call,get,call,after",
        ),
        ("absent?.run(await value())", "after,microtask"),
        (
            "absent?.[await Promise.resolve(key())](await value())",
            "after,microtask",
        ),
        ("absent?.run?.(await value())", "after,microtask"),
        ("absent?.run(await value()).run(1)", "after,microtask"),
        ("missing?.(await value())", "after,microtask"),
        ("obj?.missing?.(await value())", "after,microtask"),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!(
                r#"
const log = [];
globalThis.run = () => {{ log.push('call'); return obj; }};
const obj = {{ get run() {{ log.push('get'); return globalThis.run; }} }};
const absent = null;
const missing = undefined;
function receiver() {{ log.push('receiver'); return obj; }}
function key() {{ log.push('key'); return 'run'; }}
function value() {{ log.push('value'); return Promise.resolve(1); }}
queueMicrotask(() => log.push('microtask'));
/*#__PURE__*/ {invocation};
log.push('after');
await Promise.resolve();
console.log(log.join(','));
"#
            ),
        );
        let expected = format!("{expected}\n");
        assert_eq!(
            observe_entry(&dir.root().join("main.mjs")),
            expected.as_bytes(),
            "{invocation}"
        );
        let build = parity(&mut BuildSession::new(dir.root()).unwrap(), &options());
        assert_eq!(
            observe(&build.product.artifacts),
            expected.as_bytes(),
            "{invocation}"
        );
    }
}

#[test]
fn split_declarations_survive_warm_cache_and_source_edits_without_reparsing() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {value} from './dep.mjs'; const dead = 9, result = value + 1; console.log(result);",
    );
    write(dir.root(), "dep.mjs", "export const unused = 0, value = 1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&options()).unwrap();
    assert_eq!(cold.counters.source_parses, 2);
    assert_eq!(observe(&cold.product.artifacts), b"2\n");
    assert!(cold
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("dead") && !text.contains("unused")));
    let noop = session.build(&options()).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(warm.counters.disk_hits, 2);
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(
        (
            warm.counters.source_reparses,
            warm.counters.fragment_reparses
        ),
        (0, 0)
    );
    assert_eq!(cold.product.artifacts, warm.product.artifacts);
    write(dir.root(), "dep.mjs", "export const unused = 0, value = 2;");
    let edited = session.build(&options()).unwrap();
    assert_eq!(edited.counters.source_parses, 1);
    assert_eq!(
        (
            edited.counters.source_reparses,
            edited.counters.fragment_reparses
        ),
        (0, 0)
    );
    assert_eq!(observe(&edited.product.artifacts), b"3\n");
}

#[test]
fn parse_free_pipeline_counts_cover_cold_warm_noop_and_edits() {
    let dir = TmpDir::new();
    write(dir.root(), "entry.mjs",
        "import value from './dep.cjs'; export default value.answer; export function load(){ return import('./lazy.mjs'); }");
    write(dir.root(), "dep.cjs", "exports.answer = 42;");
    write(dir.root(), "lazy.mjs", "export const value = 7;");
    let options = BuildOptions {
        entries: [("a", "a.js"), ("z", "nested/z.mjs")]
            .into_iter()
            .map(|(id, name)| {
                (
                    id.into(),
                    Entry {
                        path: "entry.mjs".into(),
                        output_name: Some(name.into()),
                        namespace: true,
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let check = |build: &BuildResult, extractions, emitted| {
        assert_eq!(build.counters.source_parses, extractions);
        assert_eq!(
            (
                build.counters.source_reparses,
                build.counters.fragment_reparses
            ),
            (0, 0)
        );
        let executable = build
            .product
            .artifacts
            .files
            .keys()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension != "json")
            })
            .count();
        assert_eq!(
            build.counters.final_validations,
            if cfg!(debug_assertions) && emitted {
                executable
            } else {
                0
            }
        );
        assert_eq!(
            build.counters.output_parses,
            build.counters.final_validations
        );
    };
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&options).unwrap();
    check(&cold, 3, true);
    assert_eq!(observe_named(&cold.product.artifacts,
        "import * as ns from './nested/z.mjs'; console.log(ns.default, (await ns.load()).value);"), b"42 7\n");
    check(&session.build(&options).unwrap(), 0, false);
    let warm = BuildSession::new(dir.root())
        .unwrap()
        .build(&options)
        .unwrap();
    check(&warm, 0, true);
    assert_eq!(warm.counters.disk_hits, 3);
    write(dir.root(), "lazy.mjs", "export const value = 8;");
    let edited = session.build(&options).unwrap();
    check(&edited, 1, true);
    assert_eq!(
        observe_named(
            &edited.product.artifacts,
            "import * as ns from './nested/z.mjs'; console.log((await ns.load()).value);"
        ),
        b"8\n"
    );
}

#[test]
fn no_op_policy_roots_source_export_and_warm_counters() {
    let dir = TmpDir::new();
    let root = dir.root();
    write(
        root,
        "main.mjs",
        "import {value} from './dep.mjs'; console.log(value);",
    );
    write(root, "dep.mjs", "export const value = 1;");
    let mut session = BuildSession::new(root).unwrap();
    let mut options = options();
    let cold = parity(&mut session, &options);
    assert_eq!(cold.counters.source_parses, 2);
    assert_eq!(cold.counters.resolutions, 1);
    assert_eq!(cold.counters.emissions, 1);
    assert_eq!(
        cold.product.artifacts.entries["main"],
        Path::new("main.mjs")
    );
    assert_eq!(cold.product.artifacts.files.len(), 1);
    assert_eq!(observe(&cold.product.artifacts), b"1\n");
    let same = parity(&mut session, &options);
    assert_eq!(same.counters.source_parses, 0);
    assert_eq!(same.counters.resolutions, 0);
    assert_eq!(same.counters.emissions, 0);
    assert_eq!(same.counters.output_parses, 0);
    assert!(Rc::ptr_eq(&same.product, &cold.product));
    assert_eq!(same.product.generation, cold.product.generation);
    let warm = BuildSession::new(root).unwrap().build(&options).unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.counters.disk_hits, 2);
    options.effects = EffectPolicy::parse(["*.appendChild(...)"]).unwrap();
    let sidecar = segment_cache::sidecar_path(&root.join("dep.mjs"));
    let cache_before = (
        fs::read(&sidecar).unwrap(),
        fs::metadata(&sidecar).unwrap().modified().unwrap(),
    );
    let policy = parity(&mut session, &options);
    assert_eq!(policy.counters.source_parses, 0);
    assert_eq!(policy.counters.resolutions, 0);
    assert_eq!(
        policy.counters.output_parses,
        usize::from(cfg!(debug_assertions))
    );
    assert_eq!(
        (
            fs::read(&sidecar).unwrap(),
            fs::metadata(&sidecar).unwrap().modified().unwrap()
        ),
        cache_before
    );
    assert_eq!(
        policy.product.fingerprints.parse,
        cold.product.fingerprints.parse
    );
    assert_eq!(
        policy.product.fingerprints.resolution,
        cold.product.fingerprints.resolution
    );
    assert_ne!(
        policy.product.fingerprints.effects,
        cold.product.fingerprints.effects
    );
    options.entries.get_mut("main").unwrap().namespace = false;
    let roots = parity(&mut session, &options);
    assert_eq!(
        roots.product.artifacts.backend,
        crate::link::Backend::Native
    );
    assert_eq!(roots.counters.source_parses, 0);
    assert_eq!(roots.counters.resolutions, 0);
    assert_eq!(
        roots.product.fingerprints.runtime,
        cold.product.fingerprints.runtime
    );
    assert_ne!(
        roots.product.fingerprints.roots,
        policy.product.fingerprints.roots
    );
    write(root, "dep.mjs", "export const value = 2;");
    let local = parity(&mut session, &options);
    assert_eq!(local.counters.source_parses, 1);
    assert!(local.product.generation > roots.product.generation);
    assert_eq!(observe(&local.product.artifacts), b"2\n");
    assert!(local.invalidated.contains(&root.join("main.mjs")));
    write(root, "dep.mjs", "export const renamed = 2;");
    let failure = session.build(&options).err().unwrap().to_string();
    let full = BuildSession::new(root)
        .unwrap()
        .build(&options)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(failure, full);
    write(
        root,
        "dep.mjs",
        "export const value = 3; export const extra = 4;",
    );
    parity(&mut session, &options);
}

#[test]
fn delete_recreate_rename_and_old_success_after_failure() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {value} from './dep.mjs'; console.log(value);",
    );
    write(dir.root(), "dep.mjs", "export const value = 1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let options = options();
    let first = parity(&mut session, &options);
    fs::remove_file(dir.root().join("dep.mjs")).unwrap();
    assert!(session.build(&options).is_err());
    write(dir.root(), "dep.mjs", "export const value = ;");
    assert!(session.build(&options).is_err());
    write(dir.root(), "dep.mjs", "export const value = 1;");
    assert_eq!(
        parity(&mut session, &options).product.artifacts,
        first.product.artifacts
    );
    fs::rename(dir.root().join("dep.mjs"), dir.root().join("renamed.mjs")).unwrap();
    write(
        dir.root(),
        "main.mjs",
        "import {value} from './renamed.mjs'; console.log(value);",
    );
    let renamed = parity(&mut session, &options);
    assert!(renamed.product.generation > first.product.generation);
    assert!(renamed.invalidated.contains(&dir.root().join("dep.mjs")));
    assert!(renamed
        .product
        .graph
        .file_id(&dir.root().join("dep.mjs"))
        .is_none());
}

#[test]
fn runtime_intrinsics_are_captured_before_first_entry_and_reused_by_late_aliases() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "first.mjs",
        "const fail = () => {throw Error('patched');}; \
         Object.getOwnPropertyNames = fail; Object.create = fail; \
         Object.defineProperty = fail; Object.freeze = fail; Object.assign = fail; \
         Reflect.apply = fail; Array.prototype[Symbol.iterator] = fail; \
         globalThis.Object = fail; globalThis.Symbol = fail; \
         console.log(typeof __gf_cjs_0_runtime);",
    );
    write(
        dir.root(),
        "loader.mjs",
        "console.log('loader'); export function load() { return import('./dep.cjs'); }",
    );
    write(
        dir.root(),
        "dep.cjs",
        "console.log('factory'); module.exports = {value: 42};",
    );
    let mut options = options();
    options.entries = [
        ("main", "first.mjs", None),
        ("loader", "loader.mjs", None),
        ("canonical", "dep.cjs", Some("nested/canonical.cjs")),
        ("alias", "dep.cjs", Some("aliases/deep/alias.cjs")),
    ]
    .into_iter()
    .map(|(id, path, output)| {
        (
            id.into(),
            Entry {
                path: path.into(),
                output_name: output.map(PathBuf::from),
                namespace: true,
            },
        )
    })
    .collect();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let build = parity(&mut session, &options);
    assert_eq!(
        observe_named(
            &build.product.artifacts,
            "await import('./first.mjs'); \
             const loader = await import('./loader.mjs'); console.log('before'); \
             const first = await loader.load(), second = await loader.load(); \
             console.log(first === second, first.default.value); \
             const alias = await import('./nested/canonical.cjs'); \
             console.log(alias.default === first.default);"
        ),
        b"undefined\nloader\nbefore\nfactory\ntrue 42\ntrue\n"
    );
}

#[test]
fn commonjs_path_parameters_describe_the_actual_output_owner() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as paths} from './paths.cjs';",
    );
    write(
        dir.root(),
        "paths.cjs",
        "var __filename; var __dirname; module.exports = {filename: __filename, dirname: __dirname};",
    );
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    let owner = &build.product.artifacts.members[&chunks::NodeId::Segment {
        module: dir.root().join("paths.cjs"),
        index: 0,
    }];
    assert_eq!(
        observe_named(
            &build.product.artifacts,
            &format!(
                "import {{paths}} from './main.mjs'; \
                 import {{fileURLToPath}} from 'node:url'; \
                 import {{dirname}} from 'node:path'; \
                 const filename = fileURLToPath(new URL({}, import.meta.url)); \
                 console.log(paths.filename === filename, paths.dirname === dirname(filename));",
                serde_json::to_string(&format!("./{}", owner.display())).unwrap()
            )
        ),
        b"true true\n"
    );
}

#[test]
fn package_export_patterns_are_specific_across_fresh_builds() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {value} from 'pkg/feature/x'; console.log(value);",
    );
    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"type":"module","exports":{"./*":"./generic/*.mjs","./feature/*":"./specific/*.mjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/pkg/generic/feature/x.mjs",
        "export const value = 'generic';",
    );
    write(
        dir.root(),
        "node_modules/pkg/specific/x.mjs",
        "export const value = 'specific';",
    );
    let expected = observe_entry(&dir.root().join("main.mjs"));
    assert_eq!(expected, b"specific\n");
    for _ in 0..16 {
        let build = BuildSession::new(dir.root())
            .unwrap()
            .build(&options())
            .unwrap();
        assert_eq!(observe(&build.product.artifacts), expected);
    }
}

#[test]
fn package_export_conditions_match_node_for_import_and_require() {
    for conditions in [
        r#"{"node":"./node.cjs","default":"./fallback.cjs"}"#,
        r#"{"default":"./fallback.cjs","import":"./import.cjs","require":"./require.cjs"}"#,
        r#"{"import":"./import.cjs","require":"./require.cjs","default":"./fallback.cjs"}"#,
        r#"{"require":"./require.cjs","import":"./import.cjs","default":"./fallback.cjs"}"#,
        r#"{"node-addons":"./node.cjs","default":"./fallback.cjs"}"#,
        r#"{"module-sync":"./node.cjs","default":"./fallback.cjs"}"#,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import value from 'pkg'; console.log('import', value);",
        );
        write(
            dir.root(),
            "main.cjs",
            "console.log('require', require('pkg'));",
        );
        write(
            dir.root(),
            "node_modules/pkg/package.json",
            &format!(r#"{{"exports":{{".":{conditions}}}}}"#),
        );
        for value in ["node", "fallback", "import", "require"] {
            write(
                dir.root(),
                &format!("node_modules/pkg/{value}.cjs"),
                &format!("module.exports = '{value}';"),
            );
        }
        let mut options = options();
        options.entries.insert(
            "require".into(),
            Entry {
                path: "main.cjs".into(),
                output_name: None,
                namespace: true,
            },
        );
        let expected = [
            observe_entry(&dir.root().join("main.mjs")),
            observe_entry(&dir.root().join("main.cjs")),
        ]
        .concat();
        let build = BuildSession::new(dir.root())
            .unwrap()
            .build(&options)
            .unwrap();
        assert_eq!(
            observe_named(
                &build.product.artifacts,
                "await import('./main.mjs'); await import('./main.cjs');"
            ),
            expected,
            "{conditions}"
        );
    }
}

#[test]
fn package_exports_reject_private_subpaths_for_every_request_mode() {
    for (entry, source) in [
        (
            "main.mjs",
            "import value from 'pkg/private.cjs'; console.log(value);",
        ),
        (
            "main.mjs",
            "console.log((await import('pkg/private.cjs')).default);",
        ),
        ("main.cjs", "console.log(require('pkg/private.cjs'));"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), entry, source);
        write(
            dir.root(),
            "node_modules/pkg/package.json",
            r#"{"exports":{".":"./public.cjs"}}"#,
        );
        write(
            dir.root(),
            "node_modules/pkg/public.cjs",
            "module.exports = 0;",
        );
        write(
            dir.root(),
            "node_modules/pkg/private.cjs",
            "module.exports = 42;",
        );
        let mut options = options();
        options.entries.get_mut("main").unwrap().path = entry.into();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let error = format!("{:#}", session.build(&options).err().unwrap());
        assert!(error.contains("ERR_PACKAGE_PATH_NOT_EXPORTED"), "{error}");
        assert!(error.contains("pkg/private.cjs"), "{error}");
        assert!(error.contains("package.json"), "{error}");

        write(
            dir.root(),
            "node_modules/pkg/package.json",
            r#"{"exports":{".":"./public.cjs","./private.cjs":"./private.cjs"}}"#,
        );
        let allowed = session.build(&options).unwrap();
        assert_eq!(observe(&allowed.product.artifacts), b"42\n");

        write(
            dir.root(),
            "node_modules/pkg/package.json",
            r#"{"exports":{".":"./public.cjs"}}"#,
        );
        assert!(session.build(&options).is_err());

        write(dir.root(), "node_modules/pkg/package.json", "{}");
        let legacy = session.build(&options).unwrap();
        assert_eq!(observe(&legacy.product.artifacts), b"42\n");
    }
}

#[test]
fn package_exports_type_tsconfig_and_options_invalidate_independently() {
    let dir = TmpDir::new();
    let root = dir.root();
    write(
        root,
        "main.mjs",
        "import {value} from 'pkg'; console.log(value);",
    );
    write(
        root,
        "node_modules/pkg/package.json",
        r#"{"type":"module","exports":{".":{"import":"./a.js"}}}"#,
    );
    write(root, "node_modules/pkg/a.js", "export const value = 1;");
    write(root, "node_modules/pkg/b.js", "export const value = 2;");
    let mut session = BuildSession::new(root).unwrap();
    let mut options = options();
    let first = parity(&mut session, &options);
    write(
        root,
        "node_modules/pkg/package.json",
        r#"{"type":"module","exports":{".":{"import":"./b.js"}}}"#,
    );
    let package = parity(&mut session, &options);
    assert_eq!(package.counters.source_parses, 1);
    assert_eq!(package.counters.memory_hits, 1);
    assert_ne!(
        first.product.fingerprints.resolution,
        package.product.fingerprints.resolution
    );
    write(
        root,
        "node_modules/pkg/package.json",
        r#"{"type":"commonjs","exports":{".":{"import":"./b.js"}}}"#,
    );
    write(root, "node_modules/pkg/b.js", "exports.value = 3;");
    let format = parity(&mut session, &options);
    assert_eq!(format.counters.source_parses, 1);
    write(
        root,
        "main.mjs",
        "import {value} from 'choices'; console.log(value);",
    );
    write(
        root,
        "tsconfig.json",
        r#"{"compilerOptions":{"customConditions":[]}}"#,
    );
    write(
        root,
        "node_modules/choices/package.json",
        r#"{"type":"module","exports":{".":{"custom":"./b.mjs","default":"./a.mjs"}}}"#,
    );
    write(
        root,
        "node_modules/choices/a.mjs",
        "export const value = 4;",
    );
    write(
        root,
        "node_modules/choices/b.mjs",
        "export const value = 5;",
    );
    parity(&mut session, &options);
    write(
        root,
        "tsconfig.json",
        r#"{"compilerOptions":{"customConditions":["custom"]}}"#,
    );
    let tsconfig = parity(&mut session, &options);
    assert_eq!(tsconfig.counters.source_parses, 1);
    assert_eq!(tsconfig.counters.memory_hits, 1);
    options
        .resolver
        .export_conditions
        .insert(0, "custom".into());
    let config = parity(&mut session, &options);
    assert_eq!(config.counters.source_parses, 0);
    assert_eq!(config.counters.resolutions, 1);
    assert_eq!(
        config.product.fingerprints.parse,
        tsconfig.product.fingerprints.parse
    );
    assert_ne!(
        config.product.fingerprints.resolution,
        tsconfig.product.fingerprints.resolution
    );
}

#[test]
fn unsupported_configuration_is_fallible_not_silently_ignored() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "console.log('valid');");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let options = options();
    let valid = session.build(&options).unwrap();
    for json in [
        r#"{"extends":"./base.json","compilerOptions":{}}"#,
        r#"{"references":[],"compilerOptions":{}}"#,
        r#"{"compilerOptions":{"customConditions":"source"}}"#,
        r#"{"compilerOptions":{"customConditions":[1]}}"#,
        r#"{"compilerOptions":{"customConditions":[""]}}"#,
        r#"{"compilerOptions":{"customConditions":[],"customSourceConditions":[]}}"#,
    ] {
        write(dir.root(), "tsconfig.json", json);
        assert!(session.build(&options).is_err(), "{json}");
    }

    write(
        dir.root(),
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"a*b*c":["ignored"]}}}"#,
    );
    assert_eq!(
        session.build(&options).unwrap().product.artifacts,
        valid.product.artifacts
    );
    fs::remove_file(dir.root().join("tsconfig.json")).unwrap();
    assert_eq!(
        session.build(&options).unwrap().product.artifacts,
        valid.product.artifacts
    );
    write(
        dir.root(),
        ".bundler-hidden/entry.mjs",
        "console.log('hidden');",
    );
    let mut hidden = options.clone();
    hidden.entries.get_mut("main").unwrap().path = ".bundler-hidden/entry.mjs".into();
    assert!(session.build(&hidden).is_err());
    let mut browser = options.clone();
    browser.resolver.target_env = TargetEnv::Browser;
    assert!(session.build(&browser).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("main.mjs", dir.root().join("alias.mjs")).unwrap();
        assert!(session.build(&options).is_err());
    }
}

#[test]
fn async_root_add_remove_and_entry_replacement_match_full_rebuild() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = () => import('./lazy.mjs');",
    );
    write(
        dir.root(),
        "lazy.mjs",
        "export const nested = () => import('./nested.mjs');",
    );
    write(dir.root(), "nested.mjs", "export const value = 1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut options = options();
    let first = parity(&mut session, &options);
    assert_eq!(first.product.artifacts.async_facades.len(), 2);
    options.entries.insert(
        "second".into(),
        Entry {
            path: "lazy.mjs".into(),
            output_name: None,
            namespace: true,
        },
    );
    let entries = parity(&mut session, &options);
    assert_eq!(entries.counters.source_parses, 0);
    assert_eq!(entries.counters.resolutions, 0);
    assert_ne!(
        entries.product.plan.node_roots,
        first.product.plan.node_roots
    );
    options.entries.remove("second");
    write(dir.root(), "main.mjs", "export const value = 2;");
    fs::remove_file(dir.root().join("lazy.mjs")).unwrap();
    fs::remove_file(dir.root().join("nested.mjs")).unwrap();
    let removed = parity(&mut session, &options);
    assert!(removed.product.artifacts.async_facades.is_empty());
    assert_eq!(removed.product.graph.file_count(), 1);
    assert!(removed.invalidated.contains(&dir.root().join("lazy.mjs")));
}

#[test]
fn package_classification_only_edit_reparses_only_effective_mode_change() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(dir.root(), "main.mjs", "import './dep.js';");
    write(dir.root(), "dep.js", "console.log('format');");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let options = options();
    let esm = parity(&mut session, &options);
    let sidecar = segment_cache::sidecar_path(&dir.root().join("dep.js"));
    let previous = fs::read(&sidecar).unwrap();
    write(dir.root(), "package.json", r#"{"type":"commonjs"}"#);
    let cjs = parity(&mut session, &options);
    assert_eq!(cjs.counters.source_parses, 1);
    assert_eq!(cjs.counters.memory_hits, 1);
    assert_ne!(
        cjs.product.fingerprints.parse,
        esm.product.fingerprints.parse
    );
    let dep = cjs
        .product
        .graph
        .file_id(&dir.root().join("dep.js"))
        .unwrap();
    assert!(cjs.product.graph.file_facts(dep).unwrap().commonjs);
    assert_ne!(fs::read(sidecar).unwrap(), previous);
}
