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
import {value} from './shared.mjs';
globalThis.events.push('parent:' + value);
export function start() {
    return new Worker(new URL('./parser.worker', import.meta.url), {type: 'module'});
}
export const again = () => new URL('./parser.worker', import.meta.url);
"#,
    );
    write(
        dir.root(),
        "parser.worker.js",
        r#"
import {value} from './shared.mjs';
globalThis.events.push('worker:' + value);
globalThis.workerApi = {
    load: () => import('./plugin.mjs'),
    nested: () => new URL('./nested.worker.js', import.meta.url),
};
"#,
    );
    write(
        dir.root(),
        "nested.worker.js",
        "import {value} from './shared.mjs'; globalThis.events.push('nested:' + value);",
    );
    write(
        dir.root(),
        "shared.mjs",
        "globalThis.events.push('shared'); export const value = 42;",
    );
    write(dir.root(), "plugin.mjs", "export const value = 'plugin';");
}

#[test]
fn worker_entries_share_preparation_graph_and_native_multi_file_output() {
    let dir = TmpDir::new();
    fixture(&dir);
    let mut session = BuildSession::new(dir.root()).unwrap();
    let config = config();
    let first = session.build(&config).unwrap();
    assert_eq!(first.counters.source_parses, 5);
    assert_eq!(first.counters.source_reparses, 0);
    assert_eq!(first.counters.fragment_reparses, 0);
    assert_eq!(first.product.graph.module_ids().count(), 5);
    assert_eq!(
        first
            .product
            .plan
            .roots
            .keys()
            .filter(|root| matches!(root, RootId::Entry(_)))
            .count(),
        3
    );
    assert_eq!(first.product.artifacts.backend, link::Backend::Native);
    assert_eq!(first.product.artifacts.entries.len(), 3);
    assert!(first.product.artifacts.files.len() > 1);
    let worker = first
        .product
        .graph
        .file_id(&dir.root().join("parser.worker.js").canonicalize().unwrap())
        .unwrap();
    let main = first
        .product
        .graph
        .file_id(&dir.root().join("main.mjs").canonicalize().unwrap())
        .unwrap();
    assert!(!first.product.graph.edges().iter().any(
        |edge| matches!(edge.from, GraphNode::Module(file) if file == main)
            && edge.to == GraphNode::Module(worker)
    ));
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.product.artifacts, first.product.artifacts);
    assert_eq!(disk.product.graph.module_ids().count(), 5);

    observe_named(
        &first.product.artifacts,
        r#"
import assert from 'node:assert/strict';
import fs from 'node:fs';
globalThis.events = [];
globalThis.Worker = class {
    constructor(url, options) { this.url = url; this.options = options; }
};
const parent = await import('./renamed/deep/start.mjs');
assert.deepEqual(events, ['shared', 'parent:42']);
const worker = parent.start();
assert.deepEqual(worker.options, {type: 'module'});
assert.equal(worker.url.href, parent.again().href);
assert.ok(fs.existsSync(worker.url));
assert.ok(worker.url.pathname.includes('/workers/'));
assert.deepEqual(events, ['shared', 'parent:42']);
await import(worker.url);
assert.deepEqual(events, ['shared', 'parent:42', 'worker:42']);
assert.equal((await workerApi.load()).value, 'plugin');
await import(workerApi.nested());
assert.deepEqual(events, ['shared', 'parent:42', 'worker:42', 'nested:42']);
const {Worker: ThreadWorker} = await import('node:worker_threads');
async function bootRealm() {
    const bootstrap = `
        import {parentPort} from 'node:worker_threads';
        globalThis.events = [];
        await import(${JSON.stringify(worker.url.href)});
        const plugin = await workerApi.load();
        parentPort.postMessage({events, plugin: plugin.value});
        parentPort.close();
    `;
    const thread = new ThreadWorker(new URL('data:text/javascript,' + encodeURIComponent(bootstrap)), {type: 'module'});
    return new Promise((resolve, reject) => {
        thread.once('message', resolve);
        thread.once('error', reject);
        thread.once('exit', code => { if (code !== 0) reject(new Error('worker exit ' + code)); });
    });
}
const realms = await Promise.all([bootRealm(), bootRealm()]);
assert.deepEqual(realms, [
    {events: ['shared', 'worker:42'], plugin: 'plugin'},
    {events: ['shared', 'worker:42'], plugin: 'plugin'},
]);
assert.deepEqual(events, ['shared', 'parent:42', 'worker:42', 'nested:42']);
"#,
    );
}

#[test]
fn worker_entries_reuse_explicit_entries_and_revalidate_targets() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const url = new URL('./worker.js', import.meta.url);",
    );
    write(dir.root(), "worker.js", "globalThis.ready = 1;");
    let mut config = config();
    config.entries.insert(
        "explicit-worker".into(),
        Entry {
            path: "worker.js".into(),
            output_name: Some("custom/worker.mjs".into()),
            namespace: false,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    assert_eq!(first.counters.source_parses, 2);
    assert_eq!(first.product.artifacts.entries.len(), 2);
    observe_named(
        &first.product.artifacts,
        r#"
import assert from 'node:assert/strict';
import {url} from './renamed/deep/start.mjs';
assert.ok(url.pathname.endsWith('/custom/worker.mjs'));
assert.equal(globalThis.ready, undefined);
await import(url);
assert.equal(globalThis.ready, 1);
"#,
    );
    write(dir.root(), "worker.js", "globalThis.ready = 2;");
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 1);
    assert_ne!(
        edited.product.fingerprints.parse,
        first.product.fingerprints.parse
    );
    assert!(edited
        .invalidated
        .contains(&dir.root().join("worker.js").canonicalize().unwrap()));
    fs::remove_file(dir.root().join("worker.js")).unwrap();
    let error = format!(
        "{:#}",
        session.build(&config).err().expect("missing worker target")
    );
    assert!(error.contains("worker.js"), "{error}");
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &edited.product));
}

#[test]
fn worker_entries_respect_public_base_and_drop_removed_roots() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const url = new URL('./worker.js', import.meta.url);",
    );
    write(dir.root(), "worker.js", "globalThis.ready = true;");
    let mut config = config();
    config.output.base_url = "https://assets.example.invalid/build/".into();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    observe_named(
        &first.product.artifacts,
        r#"
import assert from 'node:assert/strict';
import {url} from './renamed/deep/start.mjs';
assert.ok(url.href.startsWith('https://assets.example.invalid/build/workers/'));
"#,
    );
    write(dir.root(), "main.mjs", "export const value = 42;");
    let removed = session.build(&config).unwrap();
    assert_eq!(removed.product.artifacts.entries.len(), 1);
    assert_eq!(removed.product.graph.module_ids().count(), 1);
}

#[test]
fn worker_entries_publish_empty_modules_and_reject_non_scripts() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const url = new URL('./worker.js', import.meta.url);",
    );
    write(dir.root(), "worker.js", "");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config()).unwrap();
    assert_eq!(first.product.artifacts.entries.len(), 2);
    observe_named(
        &first.product.artifacts,
        r#"
import {url} from './renamed/deep/start.mjs';
await import(url);
"#,
    );
    write(dir.root(), "asset.json", "{}");
    write(
        dir.root(),
        "main.mjs",
        "export const url = new URL('./asset.json', import.meta.url);",
    );
    let error = session.build(&config()).err().expect("non-script URL");
    let error = format!("{error:#}");
    assert!(error.contains("requires a script target"), "{error}");
    assert!(error.contains("asset.json"), "{error}");
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
}
