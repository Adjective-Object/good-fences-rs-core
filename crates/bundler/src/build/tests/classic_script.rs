use super::*;

fn classic_options() -> BuildOptions {
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.entries.get_mut("main").unwrap().namespace = false;
    config
}

fn error(session: &mut BuildSession, config: &BuildOptions) -> String {
    match session.build_classic_script(config) {
        Ok(_) => panic!("unsupported classic script succeeded"),
        Err(error) => format!("{error:#}"),
    }
}

#[test]
fn classic_script_initializes_three_import_worker_synchronously_in_isolated_realms() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import {parse} from './parser.mjs';
import {prefix} from './prefix.mjs';
import {record} from './record.mjs';
let calls = 0;
globalThis.onmessage = ({data}) => {
    record();
    globalThis.postMessage(prefix + parse(data) + ':' + ++calls);
};
globalThis.events.push('initialized');
globalThis.topLevelThis = this;
"#,
    );
    write(dir.root(), "parser.mjs", "globalThis.events.push('parser'); export function parse(value) { return value.toUpperCase(); }");
    write(
        dir.root(),
        "prefix.mjs",
        "globalThis.events.push('prefix'); export const prefix = 'parsed:';",
    );
    write(dir.root(), "record.mjs", "globalThis.events.push('record'); export function record() { globalThis.events.push('message'); }");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let config = classic_options();
    let built = session.build_classic_script(&config).unwrap();
    let artifacts = &built.product.artifacts;
    assert_eq!(artifacts.backend, link::Backend::Native);
    assert_eq!(artifacts.entries["main"], Path::new("main.js"));
    assert_eq!(artifacts.files.len(), 1);
    assert!(artifacts.modules.is_empty());
    assert_eq!(artifacts.file_bytes().count(), 1);
    assert_eq!(
        artifacts.classic_scripts,
        BTreeSet::from([PathBuf::from("main.js")])
    );
    assert_eq!(built.counters.source_reparses, 0);
    assert_eq!(built.counters.fragment_reparses, 0);
    assert_eq!(
        observe_named(
            artifacts,
            r#"
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const source = fs.readFileSync('./main.js', 'utf8');
function boot() {
    const messages = [];
    const realm = vm.createContext({events: [], postMessage: value => messages.push(value)});
    const importScripts = () => vm.runInContext(source, realm);
    importScripts();
    assert.deepEqual(Array.from(realm.events), ['parser','prefix','record','initialized']);
    assert.equal(typeof realm.onmessage, 'function');
    assert.equal(realm.topLevelThis, undefined);
    assert.equal('calls' in realm, false);
    assert.equal('parse' in realm, false);
    return {realm, messages};
}
assert.equal(globalThis.onmessage, undefined);
const first = boot();
const second = boot();
first.realm.onmessage({data: 'one'});
first.realm.onmessage({data: 'two'});
second.realm.onmessage({data: 'three'});
assert.deepEqual(first.messages, ['parsed:ONE:1','parsed:TWO:2']);
assert.deepEqual(second.messages, ['parsed:THREE:1']);
assert.deepEqual(Array.from(first.realm.events), ['parser','prefix','record','initialized','message','message']);
"#
        ),
        b""
    );
    let noop = session.build_classic_script(&config).unwrap();
    assert!(Rc::ptr_eq(&built.product, &noop.product));
    assert_eq!(noop.counters.output_parses, 0);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build_classic_script(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.disk_hits, 4);
    assert_eq!(disk.product.artifacts, *artifacts);
}

#[test]
fn classic_script_mode_changes_only_output_and_retains_native_esm_behavior() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "globalThis.ready = true; export const unused = 42;",
    );
    let config = classic_options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let native = session.build(&config).unwrap();
    let classic = session.build_classic_script(&config).unwrap();
    assert_eq!(classic.counters.source_parses, 0);
    assert_eq!(
        classic.product.fingerprints.parse,
        native.product.fingerprints.parse
    );
    assert_eq!(
        classic.product.fingerprints.resolution,
        native.product.fingerprints.resolution
    );
    assert_ne!(
        classic.product.fingerprints.output,
        native.product.fingerprints.output
    );
    assert_eq!(classic.product.artifacts.files.len(), 1);
    assert!(!classic.product.artifacts.files[Path::new("main.js")].contains("export"));
    let restored = session.build(&config).unwrap();
    assert_eq!(restored.counters.source_parses, 0);
    assert_eq!(native.product.artifacts, restored.product.artifacts);
}

#[test]
fn classic_script_rejects_unsupported_boundaries_without_replacing_last_success() {
    for (source, target, expected) in [
        ("await Promise.resolve();", "", "synchronous ESM"),
        (
            "globalThis.load = () => import('./dep.mjs');",
            "",
            "DynamicImport",
        ),
        ("import './dep.mjs';", "import './main.mjs';", "acyclic"),
        (
            "globalThis.probe = () => arguments;",
            "",
            "arguments reference",
        ),
        (
            "#!/usr/bin/env node\nglobalThis.ready = true;",
            "",
            "hashbang",
        ),
        ("import './dep.cjs';", "", "synchronous ESM"),
        ("import './style.css';", "", "script inputs only"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "globalThis.ready = true;");
        write(dir.root(), "dep.mjs", target);
        write(dir.root(), "dep.cjs", "module.exports = 42;");
        write(dir.root(), "style.css", "body {color: red}");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let config = classic_options();
        let first = session.build_classic_script(&config).unwrap();
        write(dir.root(), "main.mjs", source);
        let failure = error(&mut session, &config);
        assert!(failure.contains(expected), "{failure}");
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
        write(dir.root(), "main.mjs", "globalThis.ready = false;");
        assert!(
            session
                .build_classic_script(&config)
                .unwrap()
                .product
                .generation
                > first.product.generation
        );
    }
}

#[test]
fn classic_script_rejects_external_and_public_namespace_boundaries() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import 'external'; globalThis.ready = true;",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut config = classic_options();
    config.externals.insert("external".into());
    assert!(error(&mut session, &config).contains("external"));
    config.entries.get_mut("main").unwrap().namespace = true;
    assert!(error(&mut session, &config).contains("evaluation-only browser entry"));
    config.entries.get_mut("main").unwrap().namespace = false;
    config.resolver.target_env = TargetEnv::Node;
    assert!(error(&mut session, &config).contains("evaluation-only browser entry"));
}

#[test]
fn classic_script_output_names_and_maps_use_final_owned_paths() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "globalThis.ready = true;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut config = classic_options();
    config.output.source_maps = true;
    config.entries.get_mut("main").unwrap().output_name = Some("workers/parser.js".into());
    let built = session.build_classic_script(&config).unwrap();
    assert_eq!(
        built.product.artifacts.entries["main"],
        Path::new("workers/parser.js")
    );
    assert_eq!(
        built.product.artifacts.classic_scripts,
        BTreeSet::from([PathBuf::from("workers/parser.js")])
    );
    assert_eq!(built.product.artifacts.files.len(), 2);
    assert!(
        built.product.artifacts.files[Path::new("workers/parser.js")]
            .contains("sourceMappingURL=./parser.js.map")
    );
    assert!(built
        .product
        .artifacts
        .files
        .contains_key(Path::new("workers/parser.js.map")));
    let published = crate::publish::publish(
        &dir.root().join(".bundler-output"),
        &built.product.artifacts,
        &crate::publish::Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(published.directory.join("workers/parser.js")).unwrap(),
        built.product.artifacts.files[Path::new("workers/parser.js")].as_bytes()
    );
    assert!(!published.directory.join("workers/package.json").exists());
    config.entries.get_mut("main").unwrap().output_name = Some("workers/parser.mjs".into());
    assert!(error(&mut session, &config).contains(".js extension"));
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &built.product));
}

#[test]
fn classic_script_does_not_silence_unresolved_worker_url_dependencies() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import {CrossDomainWorker as Worker} from './wrapper.mjs';
globalThis.worker = new Worker(new URL('./worker', import.meta.url));
"#,
    );
    write(
        dir.root(),
        "wrapper.mjs",
        "export class CrossDomainWorker {}",
    );
    write(dir.root(), "worker.js", "globalThis.onmessage = () => {};");
    let mut session = BuildSession::new(dir.root()).unwrap();
    assert!(
        error(&mut session, &classic_options()).contains("resource URLs require a browser module")
    );
}
