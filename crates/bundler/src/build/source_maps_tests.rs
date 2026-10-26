use super::*;
use oxc_sourcemap::SourceMap;

fn maps(artifacts: &link::Artifacts) -> Vec<(PathBuf, SourceMap<'static>)> {
    artifacts
        .files
        .iter()
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "map"))
        .map(|(path, text)| {
            (
                path.clone(),
                SourceMap::from_json_string(text).unwrap().into_owned(),
            )
        })
        .collect()
}

fn coordinates(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 0;
    let mut column = 0;
    let mut cr = false;
    for ch in text[..offset].chars() {
        match ch {
            '\n' if cr => {}
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                line += 1;
                column = 0;
            }
            _ => column += ch.len_utf16() as u32,
        }
        cr = ch == '\r';
    }
    (line, column)
}

fn assert_origin(artifacts: &link::Artifacts, original: &Path, source: &str, needle: &str) {
    let expected = coordinates(source, source.find(needle).unwrap());
    let url = url::Url::from_file_path(original).unwrap().to_string();
    let mut found = false;
    for (map_path, map) in maps(artifacts) {
        let mut js = map_path.as_os_str().to_owned().into_string().unwrap();
        js.truncate(js.len() - 4);
        let output = &artifacts.files[Path::new(&js)];
        let table = map.generate_lookup_table();
        for (offset, _) in output.match_indices(needle) {
            let (line, column) = coordinates(output, offset);
            let Some(token) = map.lookup_source_view_token(&table, line, column) else {
                continue;
            };
            if token.get_source().is_some_and(|value| value == url) {
                assert_eq!(
                    (token.get_src_line(), token.get_src_col()),
                    expected,
                    "{needle}: {map_path:?}"
                );
                assert_eq!(token.get_source_content().unwrap(), source);
                found = true;
            }
        }
    }
    assert!(found, "missing mapped {needle} in {}", original.display());
}

#[test]
fn source_maps_follow_multispans_nested_rewrites_renames_cjs_and_async_chunks() {
    let dir = TmpDir::new();
    let sources = [
        ("main.mjs", "import {value as left} from './left.mjs'; import {value as right} from './right.mjs'; import old from './legacy.cjs'; export const load = () => import('./lazy.mjs'); console.log(left, right, old);"),
        ("left.mjs", "export const unused = 0, value = 'LEFT';\nconst obj = {method() {}}; function side(){ console.log('SIDE'); }\n/*#__PURE__*/ obj.method((/*#__PURE__*/ obj.method(side()), 1));"),
        ("right.mjs", "export const value = 'RIGHT';"),
        ("legacy.cjs", "module.exports = 'CJS';"),
        ("lazy.mjs", "export const value = 'LAZY';"),
    ];
    for (path, text) in sources {
        write(dir.root(), path, text);
    }
    let mut config = options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let build = session.build(&config).unwrap();
    let artifacts = &build.product.artifacts;
    for (path, needle) in [
        ("left.mjs", "'LEFT'"),
        ("left.mjs", "'SIDE'"),
        ("right.mjs", "'RIGHT'"),
        ("legacy.cjs", "'CJS'"),
        ("lazy.mjs", "'LAZY'"),
    ] {
        let text = sources.iter().find(|(name, _)| *name == path).unwrap().1;
        assert_origin(artifacts, &dir.root().join(path), text, needle);
    }
    let all_maps = maps(artifacts);
    for (_, map) in &all_maps {
        for token in map
            .get_source_view_tokens()
            .filter(|token| token.get_name().is_some())
        {
            let source = token.get_source_content().unwrap();
            let name = token.get_name().unwrap();
            assert!(source.contains(name));
        }
    }
    let mut parallel = config.clone();
    parallel.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(4).unwrap());
    let parallel = BuildSession::new(dir.root())
        .unwrap()
        .build(&parallel)
        .unwrap();
    assert_eq!(parallel.product.artifacts, *artifacts);
}

#[test]
fn source_maps_preserve_original_names_when_scope_hoisting_renames_bindings() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {value as left} from './left.mjs'; import {value as right} from './right.mjs'; console.log(left, right);",
    );
    write(dir.root(), "left.mjs", "export const value = 'LEFT';");
    write(dir.root(), "right.mjs", "export const value = 'RIGHT';");
    let mut config = options();
    config.output.source_maps = true;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    let mut renamed = false;
    for (path, map) in maps(&result.product.artifacts) {
        let js = path.to_str().unwrap().strip_suffix(".map").unwrap();
        let output = &result.product.artifacts.files[Path::new(js)];
        for token in map
            .get_source_view_tokens()
            .filter(|token| token.get_name().is_some_and(|name| name == "value"))
        {
            let source = token.get_source_content().unwrap();
            assert_eq!(token.get_src_line(), 0);
            assert_eq!(token.get_src_col(), source.find("value").unwrap() as u32);
            assert!(token.get_source().unwrap().ends_with(".mjs"));
            let line = output.lines().nth(token.get_dst_line() as usize).unwrap();
            // This fixture contains only ASCII; its UTF-16 column is a byte offset.
            renamed |= !line[token.get_dst_col() as usize..].starts_with("value");
        }
    }
    assert!(
        renamed,
        "expected a mapped renamed binding from a merged input"
    );
}

#[test]
fn source_maps_toggle_only_output_and_publish_final_names_and_unmapped_aliases() {
    let dir = TmpDir::new();
    let source = "#!/usr/bin/env node\nexport const value = '😀é';\r\nexport const next = value;";
    write(dir.root(), "main.mjs", source);
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("nested/a #%.mjs".into());
    let mut alias = config.entries["main"].clone();
    alias.output_name = Some("alias.mjs".into());
    config.entries.insert("other".into(), alias);
    let mut session = BuildSession::new(dir.root()).unwrap();
    let off = session.build(&config).unwrap();
    assert!(maps(&off.product.artifacts).is_empty());
    config.output.source_maps = true;
    let output = dir.root().join(".bundler-output");
    let (on, published) = session
        .build_and_publish(&config, &output, &crate::publish::Cancellation::default())
        .unwrap();
    assert_eq!(on.counters.source_parses, 0);
    assert_eq!(on.counters.source_reparses, 0);
    assert_eq!(on.counters.fragment_reparses, 0);
    assert_eq!(
        off.product.fingerprints.parse,
        on.product.fingerprints.parse
    );
    assert_ne!(
        off.product.fingerprints.output,
        on.product.fingerprints.output
    );
    let artifacts = &on.product.artifacts;
    assert_eq!(
        artifacts.files[Path::new("nested/a #%.mjs")].starts_with("#!"),
        off.product.artifacts.files[Path::new("nested/a #%.mjs")].starts_with("#!")
    );
    assert!(artifacts.files[Path::new("nested/a #%.mjs")]
        .ends_with("//# sourceMappingURL=./a%20%23%25.mjs.map\n"));
    assert_origin(artifacts, &dir.root().join("main.mjs"), source, "'😀é'");
    let alias_map =
        SourceMap::from_json_string(&artifacts.files[Path::new("alias.mjs.map")]).unwrap();
    assert_eq!(alias_map.get_sources().count(), 0);
    for (path, bytes) in artifacts.file_bytes() {
        assert_eq!(fs::read(published.directory.join(path)).unwrap(), bytes);
    }
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&on.product, &noop.product));
    assert_eq!(noop.counters.final_validations, 0);
    config.output.source_maps = false;
    let disabled = session.build(&config).unwrap();
    assert_eq!(disabled.counters.source_parses, 0);
    assert_eq!(disabled.product.artifacts, off.product.artifacts);
}

#[test]
fn source_maps_do_not_map_prepared_resource_wrappers() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import value from './data.json'; import './style.css'; import asset from './image.png'; console.log(value, asset);");
    write(dir.root(), "data.json", "{\"value\":42}");
    write(dir.root(), "style.css", "body {color:red}");
    write(dir.root(), "image.png", "image bytes");
    let mut config = options();
    config.output.source_maps = true;
    config.css.load = CssLoad::AutoMount;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    for (_, map) in maps(&result.product.artifacts) {
        assert!(map
            .get_sources()
            .all(|source| source.ends_with("/main.mjs")));
        assert!(map
            .get_source_contents()
            .flatten()
            .all(|source| source.contains("console.log(value, asset)")));
    }
}

fn upstream_json(original: &str, content: Option<&str>, line: u32) -> String {
    let mut builder = oxc_sourcemap::SourceMapBuilder::default();
    let id = builder.add_source_and_content(original, content.unwrap_or(""));
    let name = builder.add_name("originalValue");
    builder.add_token(0, 0, line, 2, Some(id), Some(name));
    let mut map = builder.into_sourcemap();
    map.set_source_contents(vec![content]);
    let mut json: serde_json::Value = serde_json::from_str(&map.to_json_string()).unwrap();
    json["sourceRoot"] = "../original".into();
    json["file"] = "compiled.mjs".into();
    json.to_string()
}

fn mapped_token(artifacts: &link::Artifacts, needle: &str) -> (SourceMap<'static>, u32, u32) {
    for (map_path, map) in maps(artifacts) {
        let js = PathBuf::from(map_path.to_string_lossy().strip_suffix(".map").unwrap());
        if let Some(offset) = artifacts.files[&js].find(needle) {
            let (line, col) = coordinates(&artifacts.files[&js], offset);
            return (map, line, col);
        }
    }
    panic!("missing mapped output {needle}");
}

#[test]
fn upstream_source_maps_compose_inline_base64_and_percent_data_with_names_and_null_contents() {
    use base64::Engine;
    for base64 in [false, true] {
        let dir = TmpDir::new();
        let json = upstream_json("input.ts", (!base64).then_some("original TypeScript"), 4);
        let data = if base64 {
            format!(
                "data:application/json;charset=utf-8;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
            )
        } else {
            format!(
                "data:application/json,{}",
                percent_encoding::utf8_percent_encode(&json, percent_encoding::NON_ALPHANUMERIC)
            )
        };
        let source = format!("export const value = 'COMPOSED';\n//# sourceMappingURL={data}");
        write(dir.root(), "main.mjs", &source);
        let mut config = options();
        config.output.source_maps = true;
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        let (map, line, col) = mapped_token(&result.product.artifacts, "'COMPOSED'");
        let table = map.generate_lookup_table();
        let token = map.lookup_source_view_token(&table, line, col).unwrap();
        assert_eq!((token.get_src_line(), token.get_src_col()), (4, 2));
        assert_eq!(token.get_name().unwrap(), "originalValue");
        let url = url::Url::from_file_path(
            dir.root()
                .parent()
                .unwrap()
                .join("original")
                .join("input.ts"),
        )
        .unwrap();
        assert_eq!(token.get_source().unwrap(), url.as_str());
        assert_eq!(
            token.get_source_content().map(AsRef::as_ref),
            (!base64).then_some("original TypeScript")
        );
        let upstream = &result.product;
        assert!(upstream
            .artifacts
            .files
            .values()
            .filter(|text| !text.starts_with('{'))
            .all(|text| !text.contains("data:application/json")));
    }
}

#[test]
fn upstream_source_maps_preserve_scheme_roots_and_prefix_leading_slash_sources() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const value = 'ROOTED';\n//# sourceMappingURL=maps/main.map",
    );
    let mut config = options();
    config.output.source_maps = true;
    let relative_url = url::Url::from_file_path(dir.root().join("original/src/input.ts")).unwrap();
    for (root, source, expected) in [
        ("webpack:///", "./src/input.ts", "webpack:///src/input.ts"),
        ("../original", "/src/input.ts", relative_url.as_str()),
    ] {
        let mut map: serde_json::Value =
            serde_json::from_str(&upstream_json(source, None, 2)).unwrap();
        map["sourceRoot"] = root.into();
        write(dir.root(), "maps/main.map", &map.to_string());
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        let (map, line, column) = mapped_token(&result.product.artifacts, "'ROOTED'");
        let lookup = map.generate_lookup_table();
        let token = map.lookup_source_view_token(&lookup, line, column).unwrap();
        assert_eq!(token.get_source().unwrap(), expected);
    }
}

#[test]
fn upstream_source_maps_linked_map_edits_invalidate_warm_and_disk_outputs_without_js_parsing() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const value = 'LINKED';\n//# sourceMappingURL=maps/main.map",
    );
    write(
        dir.root(),
        "maps/main.map",
        &upstream_json("input.ts", None, 2),
    );
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let disabled = session.build(&config).unwrap();
    config.output.source_maps = true;
    let first = session.build(&config).unwrap();
    assert_eq!(first.counters.source_parses, 0);
    assert_eq!(
        disabled.product.fingerprints.parse,
        first.product.fingerprints.parse
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    let sidecar = dir.root().join("main.mjs.upstream.seg.fb");
    let first_cache = fs::read(&sidecar).unwrap();
    // Map location, not the compiled JS location, anchors sourceRoot.
    let (map, line, col) = mapped_token(&first.product.artifacts, "'LINKED'");
    let table = map.generate_lookup_table();
    let token = map.lookup_source_view_token(&table, line, col).unwrap();
    let url = url::Url::from_file_path(dir.root().join("original/input.ts")).unwrap();
    assert_eq!(token.get_source().unwrap(), url.as_str());
    write(
        dir.root(),
        "maps/main.map",
        &upstream_json("input.ts", Some("updated source"), 8),
    );
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 0);
    assert_eq!(edited.counters.source_reparses, 0);
    assert_eq!(edited.counters.fragment_reparses, 0);
    assert_eq!(
        first.product.fingerprints.parse,
        edited.product.fingerprints.parse
    );
    assert_ne!(
        first.product.fingerprints.output,
        edited.product.fingerprints.output
    );
    assert!(edited.invalidated.contains(&dir.root().join("main.mjs")));
    assert_ne!(fs::read(&sidecar).unwrap(), first_cache);
    let (map, line, col) = mapped_token(&edited.product.artifacts, "'LINKED'");
    let table = map.generate_lookup_table();
    assert_eq!(
        map.lookup_source_view_token(&table, line, col)
            .unwrap()
            .get_src_line(),
        8
    );
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.disk_hits, 1);
    assert_eq!(edited.product.artifacts, disk.product.artifacts);
    // A corrupt map sidecar is a cold map parse, never a JS cache miss.
    fs::write(&sidecar, b"corrupt map sidecar").unwrap();
    let recovered = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(recovered.counters.disk_hits, 1);
    assert_eq!(recovered.counters.source_parses, 0);
    assert_eq!(recovered.product.artifacts, edited.product.artifacts);
    write(
        dir.root(),
        "maps/main.map",
        &upstream_json("other.ts", None, 10),
    );
    let edited_on_disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(edited_on_disk.counters.disk_hits, 1);
    assert_ne!(edited.product.artifacts, edited_on_disk.product.artifacts);
    fs::remove_file(dir.root().join("maps/main.map")).unwrap();
    let error = session.build(&config).err().unwrap();
    assert!(
        format!("{error:#}").contains("upstream source map"),
        "{error:?}"
    );
}

#[test]
fn upstream_source_maps_keep_sparse_unmapped_regions_and_synthetic_code_unmapped() {
    let dir = TmpDir::new();
    let source = "export const before = '😀MAPPED', after = 'UNMAPPED';\nconsole.log('NO_LINE');";
    let mut builder = oxc_sourcemap::SourceMapBuilder::default();
    let id = builder.add_source_and_content("original.ts", "source");
    builder.add_token(0, 0, 3, 7, Some(id), None);
    builder.add_token(
        0,
        source[..source.find("'UNMAPPED'").unwrap()]
            .encode_utf16()
            .count() as u32,
        0,
        0,
        None,
        None,
    );
    write(
        dir.root(),
        "main.map",
        &builder.into_sourcemap().to_json_string(),
    );
    write(
        dir.root(),
        "main.mjs",
        &format!("{source}\n/*# sourceMappingURL=main.map */"),
    );
    let mut config = options();
    config.output.source_maps = true;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    for (needle, mapped) in [
        ("'😀MAPPED'", true),
        ("'UNMAPPED'", false),
        ("'NO_LINE'", false),
    ] {
        let (map, line, col) = mapped_token(&result.product.artifacts, needle);
        let table = map.generate_lookup_table();
        let token = map.lookup_source_view_token(&table, line, col).unwrap();
        assert_eq!(token.get_source().is_some(), mapped, "{needle}");
    }
}

#[test]
fn upstream_source_maps_report_bad_missing_remote_and_out_of_bounds_maps_only_when_enabled() {
    let dir = TmpDir::new();
    write(dir.root(), "bad.map", "{not JSON}");
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    for (request, expected) in [
        ("bad.map", "malformed source map"),
        ("missing.map", "source map"),
        ("https://example.com/map", "unsupported remote"),
        ("//example.com/map", "unsupported remote"),
        ("file://example.com/map", "unsupported remote"),
        ("data:application/json;base64,bad!", "Invalid"),
        ("data:text/plain,{}", "unsupported inline"),
    ] {
        write(
            dir.root(),
            "main.mjs",
            &format!("export const value = 1;\n//# sourceMappingURL={request}"),
        );
        config.output.source_maps = false;
        session.build(&config).unwrap();
        config.output.source_maps = true;
        let error = session.build(&config).err().unwrap();
        assert!(
            format!("{error:#}").contains("upstream source map"),
            "{request}: {error:#}"
        );
        assert!(
            format!("{error:#}").contains(expected),
            "{request}: {error:#}"
        );
    }
    // Directive-shaped text in strings/templates is not discovery.
    write(
        dir.root(),
        "main.mjs",
        "export const value = `//# sourceMappingURL=missing.map`;",
    );
    session.build(&config).unwrap();
    write(
        dir.root(),
        "outside.map",
        &upstream_json("source.ts", None, 0),
    );
    write(
        dir.root(),
        "project/main.mjs",
        "export const value = 1;\n//# sourceMappingURL=../outside.map",
    );
    let error = BuildSession::new(dir.root().join("project"))
        .unwrap()
        .build(&config)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("source map outside build root"),
        "{error:#}"
    );
}

#[test]
fn upstream_source_maps_require_explicit_repair_of_dangling_package_directives() {
    let dir = TmpDir::new();
    let source = "module.exports = 'SHIPPED_JS';\n";
    let host_source = "exports.value = require('../bridge');\nconsole.log('SHIPPED_HOST_JS');\n";
    write(
        dir.root(),
        "main.mjs",
        "import host from './node_modules/host/index.js'; console.log(host);",
    );
    write(
        dir.root(),
        "node_modules/bridge/package.json",
        r#"{"name":"bridge","main":"index.js"}"#,
    );
    write(
        dir.root(),
        "node_modules/bridge/index.js",
        &format!("{source}//# sourceMappingURL=index.js.map\n"),
    );
    write(
        dir.root(),
        "node_modules/host/package.json",
        r#"{"name":"host","main":"index.js"}"#,
    );
    write(
        dir.root(),
        "node_modules/host/index.js",
        &format!("{host_source}//# sourceMappingURL=host.js.map\n"),
    );
    let mut config = options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let error = session.build(&config).err().unwrap();
    let report = format!("{error:#}");
    assert!(report.contains("upstream source map"), "{report}");
    assert!(report.contains("host.js.map"), "{report}");
    assert!(report.contains("missing or unsupported"), "{report}");

    // Repairing one package must not hide a dependency's missing map.
    write(dir.root(), "node_modules/host/index.js", host_source);
    let error = session.build(&config).err().unwrap();
    let report = format!("{error:#}");
    assert!(report.contains("upstream source map"), "{report}");
    assert!(report.contains("index.js.map"), "{report}");
    assert!(report.contains("missing or unsupported"), "{report}");

    write(dir.root(), "node_modules/bridge/index.js", source);
    let repaired = session.build(&config).unwrap();
    assert_origin(
        &repaired.product.artifacts,
        &dir.root().join("node_modules/bridge/index.js"),
        source,
        "'SHIPPED_JS'",
    );
    assert_origin(
        &repaired.product.artifacts,
        &dir.root().join("node_modules/host/index.js"),
        host_source,
        "'SHIPPED_HOST_JS'",
    );
    let cold = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(repaired.product.artifacts, cold.product.artifacts);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&repaired.product, &noop.product));
    config.cache = CacheMode::Disabled;
    let uncached = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(repaired.product.artifacts, uncached.product.artifacts);
}

#[test]
fn upstream_source_maps_repair_scoped_package_side_effect_subpath_without_changing_runtime() {
    let dir = TmpDir::new();
    let path = "node_modules/@vendor/runtime/src/runtime.js";
    let source = "(function(){globalThis.runtimeValue='SHIPPED_RUNTIME';})();\n";
    write(
        dir.root(),
        "main.mjs",
        "import '@vendor/runtime/src/runtime.js'; console.log(globalThis.runtimeValue);",
    );
    write(
        dir.root(),
        "node_modules/@vendor/runtime/package.json",
        r#"{"name":"@vendor/runtime","version":"1.0.0"}"#,
    );
    write(
        dir.root(),
        path,
        &format!("{source}//# sourceMappingURL=runtime.js.map\n"),
    );
    let mut config = options();
    config.output.source_maps = true;
    config.cache = CacheMode::Disabled;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let error = session.build(&config).err().unwrap();
    let report = format!("{error:#}");
    assert!(report.contains("upstream source map"), "{report}");
    assert!(report.contains("runtime.js.map"), "{report}");
    assert!(report.contains("missing or unsupported"), "{report}");

    write(dir.root(), path, source);
    let repaired = session.build(&config).unwrap();
    assert_eq!(repaired.product.artifacts.backend, link::Backend::Native);
    assert_origin(
        &repaired.product.artifacts,
        &dir.root().join(path),
        source,
        "'SHIPPED_RUNTIME'",
    );
    assert_eq!(
        observe_named(&repaired.product.artifacts, "await import('./main.mjs');"),
        b"SHIPPED_RUNTIME\n"
    );
    let cold = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(repaired.product.artifacts, cold.product.artifacts);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&repaired.product, &noop.product));
}

#[test]
fn upstream_source_maps_repair_conditional_package_barrels_and_implementations_independently() {
    for commonjs in [false, true] {
        let dir = TmpDir::new();
        let base = "node_modules/@vendor/type-safety";
        let folder = if commonjs { "lib-commonjs" } else { "lib" };
        let barrel_path = format!("{base}/{folder}/index.js");
        let implementation_path = format!("{base}/{folder}/describe.js");
        let (main, barrel, implementation) = if commonjs {
            (
                "const {describe} = require('@vendor/type-safety'); const direct = require('@vendor/type-safety/describe'); console.log(describe === direct.describe, describe(42));",
                "exports.describe = require('./describe').describe;\n",
                "exports.describe = function(value) { return 'SHIPPED_CJS_' + typeof value; };\n",
            )
        } else {
            (
                "import {describe} from '@vendor/type-safety'; import {describe as direct} from '@vendor/type-safety/describe'; console.log(describe === direct, describe(42));",
                "export * from './describe';\n",
                "export function describe(value) { return 'SHIPPED_ESM_' + typeof value; }\n",
            )
        };
        write(
            dir.root(),
            "main.mjs",
            if commonjs {
                "const {describe} = require('@vendor/type-safety'); console.log(describe(42));"
            } else {
                "import {describe} from '@vendor/type-safety'; console.log(describe(42));"
            },
        );
        write(
            dir.root(),
            &format!("{base}/package.json"),
            r#"{"name":"@vendor/type-safety","sideEffects":false,"exports":{".":{"import":"./lib/index.js","require":"./lib-commonjs/index.js"},"./describe":{"import":"./lib/describe.js","require":"./lib-commonjs/describe.js"}}}"#,
        );
        write(
            dir.root(),
            &barrel_path,
            &format!("{barrel}//# sourceMappingURL=index.js.map\n"),
        );
        write(
            dir.root(),
            &implementation_path,
            &format!("{implementation}//# sourceMappingURL=describe.js.map\n"),
        );
        let mut config = options();
        config.output.source_maps = true;
        config.cache = CacheMode::Disabled;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let report = format!("{:#}", session.build(&config).err().unwrap());
        assert!(report.contains("upstream source map"), "{report}");
        assert!(report.contains("index.js.map"), "{report}");
        assert!(report.contains("missing or unsupported"), "{report}");

        write(dir.root(), &barrel_path, barrel);
        let report = format!("{:#}", session.build(&config).err().unwrap());
        assert!(report.contains("upstream source map"), "{report}");
        assert!(report.contains("describe.js.map"), "{report}");
        assert!(report.contains("missing or unsupported"), "{report}");

        write(dir.root(), &implementation_path, implementation);
        write(dir.root(), "main.mjs", main);
        let repaired = session.build(&config).unwrap();
        assert_eq!(repaired.product.artifacts.backend, link::Backend::Native);
        let (needle, expected) = if commonjs {
            ("'SHIPPED_CJS_'", b"true SHIPPED_CJS_number\n".as_slice())
        } else {
            ("'SHIPPED_ESM_'", b"true SHIPPED_ESM_number\n".as_slice())
        };
        assert_origin(
            &repaired.product.artifacts,
            &dir.root().join(&implementation_path),
            implementation,
            needle,
        );
        assert_eq!(
            observe_named(&repaired.product.artifacts, "await import('./main.mjs');"),
            expected
        );
        let cold = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(repaired.product.artifacts, cold.product.artifacts);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&repaired.product, &noop.product));
    }
}

#[test]
fn source_maps_reserve_case_insensitive_resource_and_parent_paths() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export const value = 1;");
    let mut config = options();
    config.output.source_maps = true;
    config.entries.get_mut("main").unwrap().output_name = Some("entry.mjs".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let build = session.build(&config).unwrap();
    for collision in ["ENTRY.MJS.MAP", "entry.mjs.map/nested"] {
        let mut artifacts = link::esm_plan(
            &build.product.graph,
            &build.product.plan,
            &config.effects,
            &BTreeMap::from([(
                dir.root().join("main.mjs"),
                "export const value = 1;".into(),
            )]),
        )
        .unwrap();
        artifacts.resources.insert(collision.into(), vec![0]);
        let error = entry_outputs::apply(dir.root(), &config, &mut artifacts).unwrap_err();
        assert!(error.to_string().contains("colli"), "{error:#}");
    }
    config.entries.get_mut("main").unwrap().output_name = Some("entry.mjs.map/child.mjs".into());
    let mut artifacts = link::esm_plan(
        &build.product.graph,
        &build.product.plan,
        &config.effects,
        &BTreeMap::from([(
            dir.root().join("main.mjs"),
            "export const value = 1;".into(),
        )]),
    )
    .unwrap();
    artifacts.resources.insert("entry.mjs.map".into(), vec![0]);
    assert!(entry_outputs::apply(dir.root(), &config, &mut artifacts).is_err());
}
