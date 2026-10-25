use super::*;

fn config() -> BuildOptions {
    let mut config = options();
    config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
    config.output.source_maps = true;
    config.entries.get_mut("main").unwrap().output_name = Some("renamed/deep/start.mjs".into());
    config
}

fn fixture(dir: &TmpDir) {
    write(
        dir.root(),
        "main.mjs",
        r#"
import {CrossDomainWorker as Worker} from './wrapper.mjs';
globalThis.events.push('parent');
export function start() {
    globalThis.events.push('start');
    return new Worker(new URL('./parser.worker', import.meta.url));
}
"#,
    );
    write(
        dir.root(),
        "wrapper.mjs",
        r#"
export class CrossDomainWorker {
    constructor(url) { globalThis.events.push('wrapper'); this.url = url; }
}
"#,
    );
    write(
        dir.root(),
        "parser.worker.js",
        r#"
import {parse} from './parse.mjs';
import {prefix} from './prefix.mjs';
import {record} from './record.mjs';
let count = 0;
globalThis.events.push('worker');
onmessage = ({data}) => { record(); postMessage(prefix + parse(data) + ':' + ++count); };
"#,
    );
    write(
        dir.root(),
        "parse.mjs",
        "globalThis.events.push('parse'); export function parse(v) {return v.toUpperCase();}",
    );
    write(dir.root(), "prefix.mjs", "export const prefix = 'parsed:';");
    write(
        dir.root(),
        "record.mjs",
        "export function record() {globalThis.events.push('message');}",
    );
}

fn observe(artifacts: &Artifacts) {
    assert_eq!(
        observe_named(
            artifacts,
            r#"
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
globalThis.events = [];
const {start} = await import('./renamed/deep/start.mjs');
assert.deepEqual(events, ['parent']);
assert.equal(globalThis.onmessage, undefined);
let constructions = 0;
const NativeURL = globalThis.URL;
globalThis.URL = class extends NativeURL {
    constructor(...args) {super(...args); constructions++;}
};
const wrapped = start();
assert.equal(constructions, 1);
assert.deepEqual(events, ['parent','start','wrapper']);
assert.ok(wrapped.url instanceof URL);
assert.ok(fs.existsSync(wrapped.url));
assert.ok(wrapped.url.pathname.includes('/scripts/'));
function boot() {
    const messages = [];
    const realm = vm.createContext({events: [], onmessage: null, postMessage: v => messages.push(v)});
    const importScripts = url => vm.runInContext(fs.readFileSync(new NativeURL(url), 'utf8'), realm);
    importScripts(String(wrapped.url));
    assert.deepEqual(Array.from(realm.events), ['parse','worker']);
    assert.equal(typeof realm.onmessage, 'function');
    return {realm, messages};
}
const first = boot(), second = boot();
first.realm.onmessage({data:'one'});
first.realm.onmessage({data:'two'});
second.realm.onmessage({data:'three'});
assert.deepEqual(first.messages, ['parsed:ONE:1','parsed:TWO:2']);
assert.deepEqual(second.messages, ['parsed:THREE:1']);
assert.equal(globalThis.onmessage, undefined);
assert.deepEqual(events, ['parent','start','wrapper']);
"#
        ),
        b""
    );
}

#[test]
fn literal_url_imported_wrapper_loads_an_isolated_synchronous_classic_realm() {
    let dir = TmpDir::new();
    fixture(&dir);
    let config = config();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    assert_eq!(first.product.artifacts.classic_scripts.len(), 1);
    assert!(first
        .product
        .graph
        .file_id(&dir.root().join("parser.worker.js").canonicalize().unwrap())
        .is_none());
    assert_eq!(first.counters.source_reparses, 0);
    assert_eq!(first.counters.fragment_reparses, 0);
    observe(&first.product.artifacts);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    assert_eq!(noop.counters.output_parses, 0);
    let mut parallel = config.clone();
    parallel.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(4).unwrap());
    let parallel = session.build(&parallel).unwrap();
    assert!(Rc::ptr_eq(&first.product, &parallel.product));
    assert_eq!(parallel.counters.source_reparses, 0);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert!(disk.counters.disk_hits >= 6);
    assert_eq!(first.product.artifacts, disk.product.artifacts);
    observe(&disk.product.artifacts);
    let published = crate::publish::publish(
        &dir.root().join(".bundler-output"),
        &first.product.artifacts,
        &crate::publish::Cancellation::default(),
    )
    .unwrap();
    for (path, bytes) in first.product.artifacts.file_bytes() {
        assert_eq!(fs::read(published.directory.join(path)).unwrap(), bytes);
    }
    assert!(!published.directory.join("scripts/package.json").exists());
    let script = first.product.artifacts.classic_scripts.first().unwrap();
    let map: serde_json::Value =
        serde_json::from_str(&first.product.artifacts.files[&crate::sourcemap::path(script)])
            .unwrap();
    assert!(map["sources"].as_array().unwrap().len() >= 3);
    write(
        dir.root(),
        "parse.mjs",
        "globalThis.events.push('parse'); export function parse(v) {return 'edited:' + v;}",
    );
    let changed = session.build(&config).unwrap();
    assert_ne!(
        first.product.fingerprints.output,
        changed.product.fingerprints.output
    );
    assert_ne!(
        first.product.artifacts.files[script],
        changed.product.artifacts.files[script]
    );
    assert_eq!(changed.counters.source_parses, 1);
    assert!(changed
        .invalidated
        .contains(&dir.root().join("main.mjs").canonicalize().unwrap()));
    let failure = link::esm(
        &changed.product.graph,
        &changed.product.plan,
        &config.effects,
        &session
            .sources
            .iter()
            .map(|(path, source)| (path.clone(), source.text.clone()))
            .collect(),
    )
    .unwrap_err();
    assert!(format!("{failure:#}").contains("checked classic-script resource build"));
}

#[test]
fn resource_url_is_not_recognized_by_worker_constructor_spelling() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export function value() { return new URL('./leaf', import.meta.url); }",
    );
    write(dir.root(), "leaf.js", "export {}; globalThis.ready = true;");
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config())
        .unwrap();
    assert_eq!(built.product.artifacts.classic_scripts.len(), 1);
}

#[test]
fn unsupported_meta_and_url_shapes_remain_explicit() {
    for source in [
        "export const url = import.meta.url;",
        "new URL(request, import.meta.url);",
        "new URL('./leaf.js', import.meta['url']);",
        "new URL('./leaf.js?raw', import.meta.url);",
        "new URL('./leaf.js', import.meta.url, extra);",
        "class URL {} new URL('./leaf.js', import.meta.url);",
        "import {URL} from './wrapper.mjs'; new URL('./leaf.js', import.meta.url);",
        "new Other('./leaf.js', import.meta.url);",
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", source);
        write(dir.root(), "leaf.js", "globalThis.ready = true;");
        write(dir.root(), "wrapper.mjs", "export class URL {}");
        let failure = BuildSession::new(dir.root())
            .unwrap()
            .build(&config())
            .err()
            .unwrap();
        assert!(
            format!("{failure:#}").contains("unsupported import.meta"),
            "{failure:#}"
        );
    }
}

#[test]
fn missing_or_unsupported_url_target_never_replaces_a_successful_product() {
    let dir = TmpDir::new();
    fixture(&dir);
    let config = config();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    for target in [
        "await Promise.resolve();",
        "export const value = import('./parse.mjs');",
    ] {
        write(dir.root(), "parser.worker.js", target);
        let failure = session.build(&config).err().unwrap();
        assert!(
            format!("{failure:#}").contains("classic-script resource"),
            "{failure:#}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    }

    fs::remove_file(dir.root().join("parser.worker.js")).unwrap();
    let failure = session.build(&config).err().unwrap();
    assert!(
        format!("{failure:#}").contains("parser.worker"),
        "{failure:#}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
}

#[test]
fn script_url_uses_configured_extensions_and_final_owner_public_base() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export function value() { return new URL('./leaf.worker', import.meta.url).href; }",
    );
    write(
        dir.root(),
        "leaf.worker.js",
        "export {}; globalThis.ready = 'default';",
    );
    write(
        dir.root(),
        "leaf.worker.web.js",
        "export {}; globalThis.ready = 'web';",
    );
    for base in [
        "./",
        "assets/",
        "/static/",
        "https://cdn.example.test/resources/",
    ] {
        let mut config = config();
        config.output.base_url = base.into();
        config.resolver.extensions = vec!["web.js".into(), "js".into()];
        let built = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        let scripts = &built.product.artifacts.classic_scripts;
        assert_eq!(scripts.len(), 1);
        let script = scripts.first().unwrap();
        assert!(built.product.artifacts.files[script].contains("'web'"));
        let expected = if base == "./" || base == "assets/" {
            format!(
                "new URL({}, new URL('../../', import.meta.url)).href",
                serde_json::to_string(&format!("{base}{}", script.display())).unwrap()
            )
        } else {
            format!(
                "new URL({}, import.meta.url).href",
                serde_json::to_string(&format!("{base}{}", script.display())).unwrap()
            )
        };
        assert_eq!(
            observe_named(
                &built.product.artifacts,
                &format!(
                    r#"
    import assert from 'node:assert/strict';
    const {{value}} = await import('./renamed/deep/start.mjs');
    const expected = {};
    assert.equal(value(), expected);
    "#,
                    expected.replace(
                        "import.meta.url",
                        "new URL('./renamed/deep/start.mjs', import.meta.url).href"
                    )
                )
            ),
            b""
        );
    }
}

#[test]
fn deferred_plugin_context_is_an_explicit_classic_resource_boundary() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const url = new URL('./parser.worker', import.meta.url);",
    );
    write(dir.root(), "parser.worker.js", "import {load} from './loader.mjs'; globalThis.onmessage = async e => postMessage(await load(e.data));");
    write(
        dir.root(),
        "loader.mjs",
        "export const load = async name => await import(`./plugins/${name}`);",
    );
    write(
        dir.root(),
        "plugins/plugin.mjs",
        "export const plugin = true;",
    );
    let failure = BuildSession::new(dir.root())
        .unwrap()
        .build(&config())
        .err()
        .unwrap();
    let failure = format!("{failure:#}");
    assert!(failure.contains("classic-script resource"), "{failure}");
    assert!(failure.contains("DynamicImport"), "{failure}");
}

#[test]
fn non_js_and_nested_url_resources_are_explicit_errors() {
    for (request, input) in [
        ("./data.json", "{}"),
        (
            "./nested.mjs",
            "export const url = new URL('./leaf.mjs', import.meta.url);",
        ),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!("export const url = new URL('{request}', import.meta.url);"),
        );
        write(dir.root(), &request[2..], input);
        write(dir.root(), "leaf.mjs", "globalThis.ready = true;");
        let failure = BuildSession::new(dir.root())
            .unwrap()
            .build(&config())
            .err()
            .unwrap();
        assert!(
            format!("{failure:#}").contains("resource URL"),
            "{failure:#}"
        );
    }
}
