use super::*;

fn browser_options() -> BuildOptions {
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config
}

#[test]
fn browser_native_output_preserves_host_getters_mutations_and_scheduling() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
window;
self;
function readDocument() { return document; }
readDocument();
navigator;
location;
document.title = 'ready';
document.createElement('x-observer');
fetch('/data');
setTimeout(() => {}, 0);
addEventListener('ready', () => {});
window.customAPI();
export const load = () => import('./lazy.mjs');
"#,
    );
    write(
        dir.root(),
        "lazy.mjs",
        "document.cookie; export const value = 7;",
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&browser_options())
        .unwrap();
    assert!(result
        .product
        .artifacts
        .files
        .keys()
        .all(|path| path.extension().is_none_or(|ext| ext != "cjs")));
    assert!(
        result
            .product
            .artifacts
            .files
            .values()
            .all(|text| !text.contains("node:")),
        "{:?}",
        result.product.artifacts.files
    );
    let check = r#"
const events = [];
const doc = {
    set title(value) { events.push('title:' + value); },
    createElement(name) { events.push('element:' + name); },
    get cookie() { events.push('cookie'); return ''; }
};
for (const [name, value] of Object.entries({
    window: { customAPI() { events.push('custom'); } },
    self: {}, document: doc, navigator: {}, location: {}
})) Object.defineProperty(globalThis, name, { configurable: true, get() { events.push(name); return value; } });
globalThis.fetch = url => events.push('fetch:' + url);
globalThis.setTimeout = () => events.push('timer');
globalThis.addEventListener = name => events.push('event:' + name);
const entry = await import('./main.mjs');
events.push('loaded');
events.push((await entry.load()).value);
console.log(JSON.stringify(events));
"#;
    assert_eq!(
        observe_named(&result.product.artifacts, check),
        b"[\"window\",\"self\",\"document\",\"navigator\",\"location\",\"document\",\"title:ready\",\"document\",\"element:x-observer\",\"fetch:/data\",\"timer\",\"event:ready\",\"window\",\"custom\",\"loaded\",\"document\",\"cookie\",7]\n"
    );
}

#[test]
fn browser_target_and_custom_declarations_invalidate_prepared_environment() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "function probe() { return document; } probe();",
    );
    let mut config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let node = session.build(&config).unwrap();
    assert!(
        node.product
            .artifacts
            .files
            .values()
            .all(|text| !text.contains("document")),
        "{:?}",
        node.product.artifacts.files
    );
    config.resolver.target_env = TargetEnv::Browser;
    let browser = session.build(&config).unwrap();
    assert_eq!(browser.counters.source_parses, 1);
    assert_eq!(browser.counters.memory_hits, 0);
    assert_ne!(
        node.product.fingerprints.parse,
        browser.product.fingerprints.parse
    );
    assert_ne!(
        node.product.fingerprints.resolution,
        browser.product.fingerprints.resolution
    );
    assert!(browser
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("document")));
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&browser.product, &noop.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.disk_hits, 1);
    assert_eq!(disk.product.artifacts, browser.product.artifacts);
    config
        .declarations
        .user
        .push(ecco::parse_declarations("global symbol document { read_effects = false }").unwrap());
    let custom = session.build(&config).unwrap();
    assert_eq!(custom.counters.source_parses, 1);
    assert!(custom
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("document")));
    config.declarations.user.clear();
    write(dir.root(), "main.mjs", "document; Math.abs(424242);");
    config.declarations.browser = false;
    config.declarations.javascript = false;
    let disabled = session.build(&config).unwrap();
    assert!(disabled
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("document")));
    assert!(disabled
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("424242")));
    config.declarations = ecco::DeclarationConfig::default();
    config.resolver.target_env = TargetEnv::Node;
    write(
        dir.root(),
        "main.mjs",
        "function probe() { return document; } probe();",
    );
    let restored = session.build(&config).unwrap();
    assert_eq!(restored.product.artifacts, node.product.artifacts);
    assert_eq!(restored.product.fingerprints, node.product.fingerprints);
}

#[test]
fn browser_prunes_pure_expressions_but_preserves_deferred_host_and_wrapper_reads() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "Math.abs(424242);");
    let mut config = browser_options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let browser = session.build(&config).unwrap();
    write(dir.root(), "main.mjs", "document;");
    let host = session.build(&config).unwrap();
    assert!(browser
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("424242")));
    assert!(host
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("document")));
    write(
        dir.root(),
        "main.mjs",
        "function probe() { return typeof module; } probe();",
    );
    let wrapper = session.build(&config).unwrap();
    assert_eq!(observe_named(&wrapper.product.artifacts,
        "Object.defineProperty(globalThis, 'module', { get() { console.log('wrapper'); } }); await import('./main.mjs');"),
        b"wrapper\n");
    config
        .declarations
        .user
        .push(ecco::parse_declarations("global symbol module { read_effects = false }").unwrap());
    let overridden = session.build(&config).unwrap();
    assert!(overridden
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("typeof module")));
}

#[test]
fn browser_package_conditions_and_browser_field_follow_target_switches() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs",
        "import {value} from 'conditional'; import {field} from 'legacy'; import {mapped} from 'mapped'; console.log(value, field, mapped);");
    write(
        dir.root(),
        "node_modules/conditional/package.json",
        r#"{"exports":{".":{"custom":"./custom.mjs","node":"./node.mjs","browser":"./browser.mjs","default":"./fallback.mjs"}}}"#,
    );
    for name in ["node", "browser", "fallback", "custom"] {
        write(
            dir.root(),
            &format!("node_modules/conditional/{name}.mjs"),
            &format!("export const value = '{name}';"),
        );
    }
    write(
        dir.root(),
        "node_modules/legacy/package.json",
        r#"{"main":"./node.mjs","browser":"./browser.mjs"}"#,
    );
    write(
        dir.root(),
        "node_modules/legacy/node.mjs",
        "export const field = 'node-field';",
    );
    write(
        dir.root(),
        "node_modules/legacy/browser.mjs",
        "export const field = 'browser-field';",
    );
    write(
        dir.root(),
        "node_modules/mapped/package.json",
        r#"{"main":"server.mjs","browser":{"./server.mjs":"./browser.mjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/mapped/server.mjs",
        "export const mapped = 'server';",
    );
    write(
        dir.root(),
        "node_modules/mapped/browser.mjs",
        "export const mapped = 'browser';",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let mut config = options();
    for (target, expected) in [
        (TargetEnv::Node, "node node-field server\n"),
        (TargetEnv::Browser, "browser browser-field browser\n"),
        (TargetEnv::Node, "node node-field server\n"),
    ] {
        config.resolver.target_env = target;
        let result = session.build(&config).unwrap();
        assert_eq!(observe(&result.product.artifacts), expected.as_bytes());
        let fresh = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(fresh.product.artifacts, result.product.artifacts);
    }
    config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
    config.resolver.export_conditions.push("custom".into());
    let custom = session.build(&config).unwrap();
    assert_eq!(
        observe(&custom.product.artifacts),
        b"custom browser-field browser\n"
    );
}

#[test]
fn browser_rejects_node_builtins_even_if_external_and_node_default_still_accepts_them() {
    for request in ["node:fs", "fs", "node:not-a-real-builtin"] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", &format!("import '{request}';"));
        let mut config = browser_options();
        let mut session = BuildSession::new(dir.root()).unwrap();
        for external in [false, true] {
            if external {
                config.externals.insert(request.into());
            }
            let error = session.build(&config).err().unwrap();
            let error = format!("{error:#}");
            assert!(
                error.contains("Node builtin")
                    && error.contains("browser target")
                    && error.contains(request),
                "{error}"
            );
        }
        if request == "node:fs" {
            config.resolver.target_env = TargetEnv::Node;
            let node = session.build(&config).unwrap();
            assert_eq!(observe(&node.product.artifacts), b"");
        }
    }
}

#[test]
fn browser_rejects_node_loader_only_output_instead_of_emitting_broken_chunks() {
    for (file, source, entry, diagnostic) in [
        (
            "dep.cjs",
            "exports.value = 1;",
            "import './dep.cjs';",
            "CommonJS",
        ),
        (
            "parts/lazy.cjs",
            "exports.value = 1;",
            "export const load = name => import(`./parts/${name}.cjs`);",
            "CommonJS",
        ),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), file, source);
        write(dir.root(), "main.mjs", entry);
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&browser_options())
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains(diagnostic), "{error:#}");
    }
}

fn assert_browser_artifacts(artifacts: &Artifacts) {
    for (path, text) in &artifacts.files {
        assert_ne!(path.extension().and_then(|ext| ext.to_str()), Some("cjs"));
        for unsupported in ["node:", "module.exports", "require("] {
            assert!(!text.contains(unsupported), "{}: {text}", path.display());
        }
    }
}

#[test]
fn browser_resources_are_native_shared_values_with_relocated_assets() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import data from './data.json';
import same from './same.mjs';
import asset from './image.svg';
import style from './global.css';
import {button} from './style.module.css';
export {data, same, asset, style, button};
export const load = () => import('./data.json');
"#,
    );
    write(
        dir.root(),
        "same.mjs",
        "export {default} from './data.json';",
    );
    write(
        dir.root(),
        "data.json",
        r#"{"__proto__":{"safe":true},"nested":{"value":1},"dup":1,"dup":2}"#,
    );
    write(dir.root(), "image.svg", "<svg/>");
    write(
        dir.root(),
        "global.css",
        "body { background: url('./image.svg') }",
    );
    write(dir.root(), "style.module.css", ".button { color: red }");
    let mut config = browser_options();
    config.css.css_modules = CssModules::Enabled;
    config.entries.get_mut("main").unwrap().output_name = Some("nested/entry.mjs".into());
    config.output.base_url = "/static/".into();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&config).unwrap();
    assert_browser_artifacts(&result.product.artifacts);
    assert!(result
        .product
        .artifacts
        .resources
        .values()
        .any(|bytes| bytes == b"<svg/>"));
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
globalThis.document = new Proxy({}, {get() {throw new Error('caller-loaded CSS touched DOM');}});
const {data, same, asset, style, button, load} = await import('./nested/entry.mjs');
if (data !== same || data !== (await load()).default) throw new Error('identity');
data.nested.value = 8;
if ((await load()).default.nested.value !== 8) throw new Error('shared mutation');
if (!Object.hasOwn(data, '__proto__') || !data.__proto__.safe || Object.getPrototypeOf(data) !== Object.prototype || data.dup !== 2) throw new Error('JSON keys');
if (!asset.startsWith('/static/assets/') || !asset.endsWith('.svg') || !style.startsWith('/static/assets/') || !style.endsWith('.css')) throw new Error('URLs');
if (!/^_[a-zA-Z0-9_-]{10}$/.test(button)) throw new Error('CSS names');
console.log('resources');
"#
        ),
        b"resources\n"
    );
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.product.artifacts, result.product.artifacts);
    assert_eq!(disk.counters.source_parses, 0);
    config.resolver.target_env = TargetEnv::Node;
    let node = session.build(&config).unwrap();
    assert!(node
        .product
        .artifacts
        .files
        .keys()
        .any(|path| path.extension().is_some_and(|ext| ext == "cjs")));
    config.resolver.target_env = TargetEnv::Browser;
    assert_eq!(
        session.build(&config).unwrap().product.artifacts,
        result.product.artifacts
    );
}

#[test]
fn browser_contexts_preserve_lazy_mounts_arguments_and_captured_errors() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import './early.css';
export const load = (name, Error, __gf_cjs_0_runtime) => import(`./parts/${name()}.mjs`);
export const style = name => import(`./styles/${name}.css`);
"#,
    );
    write(dir.root(), "early.css", "body { color: red }");
    write(
        dir.root(),
        "parts/a.mjs",
        "globalThis.events.push('lazy'); export const value = 7;",
    );
    write(dir.root(), "styles/late.css", "body { color: blue }");
    let mut config = browser_options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("nested/entry.mjs".into());
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_browser_artifacts(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
globalThis.events = [];
globalThis.document = {
  createElement() { return {}; },
  head: {appendChild(link) {
    if (link.rel !== 'stylesheet' || !link.href.startsWith(new URL('./assets/', import.meta.url).href)) throw new Error('relocation');
    events.push('mount');
  }}
};
const NativeError = Error;
const entry = await import('./nested/entry.mjs');
events.push('entry');
globalThis.Error = function () { throw new NativeError('uncaptured Error'); };
let calls = 0;
const argument = () => { calls++; return 'a'; };
const first = entry.load(argument, null, null);
events.push('requested');
if ((await first).value !== 7 || (await first) !== await entry.load(argument)) throw new NativeError('namespace');
const miss = entry.load(() => {calls++; return 'missing';}, null, null);
if (!(miss instanceof Promise) || calls !== 3) throw new NativeError('arguments');
events.push('miss-returned');
try { await miss; throw new NativeError('unexpected success'); }
catch (error) {
  if (!(error instanceof NativeError) || error.code !== 'ERR_MODULE_NOT_FOUND' || !error.message.includes('./parts/missing.mjs')) throw error;
  events.push('rejected');
}
await entry.style('late');
await entry.style('late');
console.log(JSON.stringify(events));
"#
        ),
        b"[\"mount\",\"entry\",\"requested\",\"lazy\",\"miss-returned\",\"rejected\",\"mount\"]\n"
    );
}
