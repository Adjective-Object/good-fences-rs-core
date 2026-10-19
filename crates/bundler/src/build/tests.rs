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
            let source = fs::read_to_string(&path).unwrap();
            (path, source)
        })
        .collect();
    let mut checked =
        link::esm(graph, &incremental.product.plan, &options.effects, &sources).unwrap();
    entry_outputs::apply(session.root(), options, &mut checked).unwrap();
    assert_eq!(incremental.product.artifacts, checked);
    incremental
}

fn observe(artifacts: &Artifacts) -> Vec<u8> {
    let dir = TmpDir::new();
    for (path, text) in &artifacts.files {
        write(dir.root(), path.to_str().unwrap(), text);
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
    for (path, source) in &artifacts.files {
        write(dir.root(), path.to_str().unwrap(), source);
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
    assert!(policy.counters.output_parses > 0);
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
        "import {value} from '@value'; console.log(value);",
    );
    write(
        root,
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@value":["a.mjs"]}}}"#,
    );
    write(root, "a.mjs", "export const value = 4;");
    write(root, "b.mjs", "export const value = 5;");
    parity(&mut session, &options);
    write(
        root,
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@value":["b.mjs"]}}}"#,
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
        r#"{"compilerOptions":{"paths":{"a*b*c":["x"]}}}"#,
        r#"{"compilerOptions":{"paths":{"a*b":["x"]}}}"#,
        r#"{"compilerOptions":{"paths":{"a/*":["x"],"a/sub/*":["y"]}}}"#,
        r#"{"compilerOptions":{"paths":{"a/*":["x/*/*"]}}}"#,
        r#"{"compilerOptions":{"paths":{"exact":["x/*"]}}}"#,
    ] {
        write(dir.root(), "tsconfig.json", json);
        assert!(session.build(&options).is_err(), "{json}");
    }

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
