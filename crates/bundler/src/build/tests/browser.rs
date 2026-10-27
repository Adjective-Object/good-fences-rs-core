use super::*;

fn browser_options() -> BuildOptions {
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config
}

#[test]
fn json_import_attributes_share_native_values_and_checked_cache_hits() {
    let dir = TmpDir::new();
    write(dir.root(), "data.json", r#"{"primitives":{"fog":7}}"#);
    write(
        dir.root(),
        "reader.cjs",
        "module.exports = () => require('./data.json');",
    );
    write(
        dir.root(),
        "main.mjs",
        "import mappings from 'theme-data' with {type: 'json'};
         import {default as attributed} from './data.json' with {'type': 'json'};
         import {default as plain} from './data.json';
         import read from './reader.cjs';
         const dynamic = await import('./data.json');
         mappings.changed = 3;
         console.log(mappings === attributed, mappings === plain,
             mappings === read(), mappings === dynamic.default,
             read().changed, mappings.primitives.fog);",
    );
    for target in [TargetEnv::Browser, TargetEnv::Node] {
        let mut config = browser_options();
        config.resolver.target_env = target;
        config
            .resolver
            .alias
            .insert("theme-data".into(), "./data.json".into());
        config.output.source_maps = true;
        config.entries.get_mut("main").unwrap().output_name = Some("nested/theme.mjs".into());
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert_eq!(cold.product.artifacts.backend, crate::link::Backend::Native);
        assert_eq!(
            observe(&cold.product.artifacts),
            b"true true true true 3 7\n"
        );
        assert!(cold
            .product
            .artifacts
            .files
            .keys()
            .any(|path| path.to_string_lossy().ends_with(".map")));
        assert!(cold
            .product
            .artifacts
            .files
            .iter()
            .filter(|(path, _)| path
                .extension()
                .is_some_and(|ext| ext == "mjs" || ext == "cjs"))
            .all(|(_, text)| !text.contains("with {")));
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.counters.disk_hits, 3);
        assert_eq!(warm.product.artifacts, cold.product.artifacts);
        config.cache = CacheMode::Disabled;
        let uncached = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(uncached.product.artifacts, cold.product.artifacts);
        write(dir.root(), "data.json", r#"{"primitives":{"fog":9}}"#);
        let edited = session.build(&config).unwrap();
        assert_eq!(
            observe(&edited.product.artifacts),
            b"true true true true 3 9\n"
        );
        write(dir.root(), "data.json", r#"{"primitives":{"fog":7}}"#);
    }
}

#[test]
fn json_import_attributes_reject_non_json_aliases_externals_and_css_overrides() {
    let dir = TmpDir::new();
    write(dir.root(), "data.json", r#"{"value":7}"#);
    write(dir.root(), "script.mjs", "export default {value: 7};");
    write(dir.root(), "data.css", ".value { color: red; }");
    write(dir.root(), "data.bin", "opaque");
    write(
        dir.root(),
        "main.mjs",
        "import data from 'theme-data' with {type: 'json'}; export default data;",
    );
    let mut config = browser_options();
    config
        .resolver
        .alias
        .insert("theme-data".into(), "./data.json".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let good = session.build(&config).unwrap();
    for target in ["./script.mjs", "./data.css", "./data.bin"] {
        config
            .resolver
            .alias
            .insert("theme-data".into(), target.into());
        let error = format!("{:#}", session.build(&config).err().unwrap());
        assert!(
            error.contains("requires a bundled JSON resource"),
            "{error}"
        );
        assert!(
            error.contains("main.mjs")
                && error.contains("theme-data")
                && error.contains("bytes 0.."),
            "{error}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &good.product));
    }
    config.resolver.alias.clear();
    config.externals.insert("theme-data".into());
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("requires a bundled JSON resource"),
        "{error}"
    );
    config.externals.clear();
    config
        .resolver
        .alias
        .insert("theme-data".into(), "./data.json".into());
    config.loaders = global_styles()
        .with_override(".json", Some(crate::loaders::Loader::CssModules))
        .unwrap();
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("requires a bundled JSON resource"),
        "{error}"
    );
}

#[test]
fn json_import_attributes_scope_hoist_without_changing_native_source_results() {
    let dir = TmpDir::new();
    write(dir.root(), "data.json", r#"{"primitives":{"fog":7}}"#);
    write(
        dir.root(),
        "main.mjs",
        "import mappings from './data.json' /* keep coordinates */ with {type: 'json'};
         import './data.json' with {'type': 'json'};
         export const theme = Object.fromEntries(Object.entries(mappings.primitives));
         console.log(JSON.stringify(theme));",
    );
    let original = observe_entry(&dir.root().join("main.mjs"));
    assert_eq!(original, b"{\"fog\":7}\n");
    let mut config = browser_options();
    config.output.source_maps = true;
    config.cache = CacheMode::Disabled;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        result.product.artifacts.backend,
        crate::link::Backend::Native
    );
    assert_eq!(observe(&result.product.artifacts), original);
    assert_eq!(result.counters.source_reparses, 0);
    assert_eq!(result.counters.fragment_reparses, 0);
}

#[test]
fn browser_eval_identity_tables_preserve_native_values_and_cache_reuse() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "primordials.mjs",
        "export const Primordials = Object.freeze((function () {
            const p = [Array, eval, Function, Object];
            return p.concat(p.filter(i => i.name[0].toUpperCase() === i.name[0])
                .map(k => k.prototype));
        })());
        export function read() { return eval; }",
    );
    write(
        dir.root(),
        "legacy.cjs",
        "module.exports = Object.freeze([Array, eval, Function]);",
    );
    write(
        dir.root(),
        "main.mjs",
        "export {Primordials, read} from './primordials.mjs';
         export {default as legacy} from './legacy.cjs';",
    );
    let mut config = browser_options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(
        built.product.artifacts.backend,
        crate::link::Backend::Native
    );
    assert_eq!(built.product.graph.file_count(), 3);
    assert!(session.sources.values().all(|source| source
        .record
        .module()
        .facts()
        .dependencies()
        .iter()
        .all(|dependency| dependency.kind() != ast_segmenter::owned::DependencyKind::Require)));
    let entry = built.product.artifacts.entries["main"]
        .to_string_lossy()
        .replace('\\', "/");
    for (host, expected) in [
        ("", r#"[[true,true,true,true,true,true,7],[]]"#),
        (
            "Object.defineProperty(globalThis, 'eval', {get() { events.push('read'); return intrinsic; }});",
            r#"[[true,true,true,true,true,true,7],["read","read","read"]]"#,
        ),
        (
            "Object.defineProperty(globalThis, 'eval', {get() { events.push('read'); throw new RangeError('host failure'); }});",
            r#"["RangeError",["read"]]"#,
        ),
        ("delete globalThis.eval;", r#"["ReferenceError",[]]"#),
    ] {
        let check = |entry: &str| format!(
            "const intrinsic = globalThis.eval;
             const events = [];
             {host}
             let result;
             try {{
                 const ns = await import('./{entry}');
                 result = [ns.Primordials[1] === intrinsic, ns.legacy[1] === intrinsic,
                     ns.read() === intrinsic, Object.isFrozen(ns.Primordials),
                     Object.isFrozen(ns.legacy), ns.Primordials[4] === Array.prototype,
                     ns.Primordials.length];
             }} catch (error) {{ result = error.name; }}
             console.log(JSON.stringify([result, events]));"
        );
        write(dir.root(), "check.mjs", &check("main.mjs"));
        let original = observe_entry(&dir.root().join("check.mjs"));
        assert_eq!(String::from_utf8_lossy(&original).trim(), expected);
        assert_eq!(
            observe_named(&built.product.artifacts, &check(&entry)),
            original,
            "{host}"
        );
    }
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&built.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.disk_hits, 3);
    assert_eq!(disk.product.artifacts, built.product.artifacts);
    config.cache = CacheMode::Disabled;
    let uncached = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(uncached.product.artifacts, built.product.artifacts);
}

#[test]
fn browser_esm_environment_probes_preserve_conditional_host_loader_selection() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "names.mjs",
        "export const require = 'local', __webpack_require__ = 'webpack-local',
            __non_webpack_require__ = 'non-webpack-local';",
    );
    write(
        dir.root(),
        "client.mjs",
        r#"
export class FetchHttpClient {
    constructor() {
        if (typeof fetch === 'undefined') {
            const requireFunc = typeof __webpack_require__ === 'function' ? __non_webpack_require__ : require;
            this.fetchType = requireFunc('node-fetch');
        } else {
            this.fetchType = fetch;
        }
        if (typeof AbortController === 'undefined') {
            const requireFunc = typeof __webpack_require__ === 'function' ? __non_webpack_require__ : require;
            this.abortControllerType = requireFunc('abort-controller');
        } else {
            this.abortControllerType = AbortController;
        }
    }
}
"#,
    );
    write(
        dir.root(),
        "main.mjs",
        "import {require, __webpack_require__, __non_webpack_require__} from './names.mjs';
         import {FetchHttpClient} from './client.mjs';
         export function run() {
             const client = new FetchHttpClient();
             return [require, __webpack_require__, __non_webpack_require__,
                 client.fetchType(), client.abortControllerType()];
         }",
    );
    let mut config = browser_options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(
        built.product.artifacts.backend,
        crate::link::Backend::Native
    );
    assert_eq!(built.product.graph.file_count(), 3);
    assert!(session.sources.values().all(|source| {
        !source.record.config().commonjs()
            && source
                .record
                .module()
                .facts()
                .dependencies()
                .iter()
                .all(|dependency| {
                    dependency.kind() != ast_segmenter::owned::DependencyKind::Require
                })
    }));
    let entry = built.product.artifacts.entries["main"]
        .to_string_lossy()
        .replace('\\', "/");
    for (host, expected) in [
        (
            "globalThis.fetch = () => 'browser-fetch';
             globalThis.AbortController = () => 'browser-abort';",
            r#"[["local","webpack-local","non-webpack-local","browser-fetch","browser-abort"],[]]"#,
        ),
        (
            "",
            r#"["ReferenceError",["webpack"]]"#,
        ),
        (
            "Object.defineProperty(globalThis, 'require', {get() { events.push('require'); return loader; }});",
            r#"[["local","webpack-local","non-webpack-local","node-fetch","abort-controller"],["webpack","require","node-fetch","webpack","require","abort-controller"]]"#,
        ),
        (
            "webpack = loader;",
            r#"["ReferenceError",["webpack"]]"#,
        ),
        (
            "webpack = loader;
             Object.defineProperty(globalThis, '__non_webpack_require__', {get() { events.push('non-webpack'); return loader; }});
             Object.defineProperty(globalThis, 'require', {get() { throw new Error('wrong loader'); }});",
            r#"[["local","webpack-local","non-webpack-local","node-fetch","abort-controller"],["webpack","non-webpack","node-fetch","webpack","non-webpack","abort-controller"]]"#,
        ),
        (
            "globalThis.fetch = () => 'browser-fetch';
             Object.defineProperty(globalThis, 'require', {get() { events.push('require'); return loader; }});",
            r#"[["local","webpack-local","non-webpack-local","browser-fetch","abort-controller"],["webpack","require","abort-controller"]]"#,
        ),
        (
            "Object.defineProperty(globalThis, 'require', {get() { events.push('require'); throw new RangeError('host failure'); }});",
            r#"["RangeError",["webpack","require"]]"#,
        ),
    ] {
        let check = |entry: &str| format!(
            "const events = [];
             let webpack;
             const loader = name => {{ events.push(name); return () => name; }};
             globalThis.fetch = undefined;
             globalThis.AbortController = undefined;
             Object.defineProperty(globalThis, '__webpack_require__', {{get() {{ events.push('webpack'); return webpack; }} }});
             {host}
             const entry = await import('./{entry}');
             let result;
             try {{ result = entry.run(); }} catch (error) {{ result = error.name; }}
             console.log(JSON.stringify([result, events]));"
        );
        write(dir.root(), "check.mjs", &check("main.mjs"));
        let original = observe_entry(&dir.root().join("check.mjs"));
        assert_eq!(String::from_utf8_lossy(&original).trim(), expected);
        assert_eq!(
            observe_named(&built.product.artifacts, &check(&entry)),
            original,
            "{host}"
        );
    }
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&built.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.disk_hits, 3);
    assert_eq!(disk.product.artifacts, built.product.artifacts);
    config.cache = CacheMode::Disabled;
    let uncached = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(uncached.product.artifacts, built.product.artifacts);
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
fn browser_bare_builtin_package_bundles_native_cjs_dependencies_and_revalidates() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {Buffer} from 'buffer'; export {Buffer}; export const load = () => import('./lazy.cjs');",
    );
    write(
        dir.root(),
        "lazy.cjs",
        "events.push('lazy'); module.exports = require('buffer').Buffer;",
    );
    write(
        dir.root(),
        "node_modules/buffer/package.json",
        r#"{"name":"buffer","main":"index.js","dependencies":{"base64-js":"1.0.0","ieee754":"1.0.0"}}"#,
    );
    write(
        dir.root(),
        "node_modules/buffer/index.js",
        "events.push('buffer'); const base = require('base64-js'); const ieee = require('ieee754'); exports.Buffer = {value: base.value + ieee.value};",
    );
    for (name, value) in [("base64-js", 20), ("ieee754", 22)] {
        write(
            dir.root(),
            &format!("node_modules/{name}/package.json"),
            &format!(r#"{{"name":"{name}","main":"index.js"}}"#),
        );
        write(
            dir.root(),
            &format!("node_modules/{name}/index.js"),
            &format!("events.push('{name}'); exports.value = {value};"),
        );
    }
    let mut config = browser_options();
    // Even an external declaration cannot turn a builtin spelling into host transport.
    config.externals.insert("buffer".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    assert_eq!(
        first.product.artifacts.backend,
        crate::link::Backend::Native
    );
    assert_browser_transport(&first.product.artifacts);
    let check = |value| {
        format!(
            r#"
if (typeof require !== 'undefined' || typeof module !== 'undefined') throw new Error('host loader');
globalThis.events = [];
const main = await import('./main.mjs');
if (main.Buffer.value !== {value}) throw new Error('package value');
if (events.join() !== 'buffer,base64-js,ieee754') throw new Error('activation: ' + events);
const lazy = await main.load();
if (lazy.default !== main.Buffer || (await main.load()).default !== main.Buffer) throw new Error('cache identity');
console.log(events.join());
"#
        )
    };
    let expected = b"buffer,base64-js,ieee754,lazy\n";
    assert_eq!(
        observe_named(&first.product.artifacts, &check(42)),
        expected
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.product.artifacts, first.product.artifacts);
    assert_eq!(observe_named(&disk.product.artifacts, &check(42)), expected);
    write(
        dir.root(),
        "node_modules/buffer/package.json",
        r#"{"name":"buffer","main":"next.cjs","dependencies":{"base64-js":"1.0.0","ieee754":"1.0.0"}}"#,
    );
    write(
        dir.root(),
        "node_modules/buffer/next.cjs",
        "events.push('buffer'); const base = require('base64-js'); const ieee = require('ieee754'); exports.Buffer = {value: base.value + ieee.value + 1};",
    );
    let changed = session.build(&config).unwrap();
    assert_browser_transport(&changed.product.artifacts);
    assert_eq!(
        observe_named(&changed.product.artifacts, &check(43)),
        expected
    );
    std::fs::remove_file(dir.root().join("node_modules/buffer/next.cjs")).unwrap();
    std::fs::remove_file(dir.root().join("node_modules/buffer/index.js")).unwrap();
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("buffer") && error.contains("browser target"),
        "{error}"
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
fn browser_commonjs_chains_cycles_retry_and_deferred_contexts() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './api.cjs';
export {api};
export const dynamic = name => import(`./parts/${name}.cjs`);
"#,
    );
    write(
        dir.root(),
        "api.cjs",
        r#"
events.push('api');
exports.load = (name, __gf_cjs_0_runtime) => require(`./parts/${name()}.cjs`);
exports.retry = () => require('./retry.cjs');
exports.cycle = () => require('./a.cjs');
"#,
    );
    write(
        dir.root(),
        "parts/lazy.cjs",
        r#"
events.push('lazy');
exports.old = true;
const original = exports;
module.exports = {value: 7, original, sameThis: this === original};
exports.detached = true;
"#,
    );
    write(
        dir.root(),
        "a.cjs",
        r#"
events.push('a'); exports.early = 1;
exports.b = require('./b.cjs'); exports.done = 2;
"#,
    );
    write(
        dir.root(),
        "b.cjs",
        r#"
events.push('b'); exports.partial = require('./a.cjs').early;
exports.incomplete = require('./a.cjs').done === undefined;
"#,
    );
    write(
        dir.root(),
        "retry.cjs",
        r#"
events.push('retry');
globalThis.attempts = (globalThis.attempts || 0) + 1;
if (globalThis.attempts === 1) throw new Error('first');
module.exports = {ok: true};
"#,
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&browser_options())
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
if (typeof require !== 'undefined' || typeof module !== 'undefined') throw new Error('host loader');
globalThis.events = [];
const {api, dynamic} = await import('./main.mjs');
if (events.join() !== 'api') throw new Error('eager factory');
let calls = 0;
const value = api.load(() => {calls++; return 'lazy';}, null);
if (!value.sameThis || !value.original.detached || value.old !== undefined) throw new Error('aliases');
if (value !== api.load(() => {calls++; return 'lazy';})) throw new Error('cache');
const ns = await dynamic('lazy');
if (ns.default !== value || ns.value !== 7) throw new Error('dynamic identity');
const NativeError = Error;
globalThis.Error = function () { throw new NativeError('uncaptured Error'); };
try { api.load(() => {calls++; return 'missing';}); throw new NativeError('missing succeeded'); }
catch (e) { if (!(e instanceof NativeError) || e.code !== 'MODULE_NOT_FOUND') throw e; }
globalThis.Error = NativeError;
if (calls !== 3) throw new Error('argument evaluated more than once');
const cycle = api.cycle();
if (cycle.b.partial !== 1 || !cycle.b.incomplete || cycle.done !== 2 || api.cycle() !== cycle) throw new Error('cycle');
try { api.retry(); throw new Error('retry succeeded'); } catch (e) { if (e.message !== 'first') throw e; }
if (!api.retry().ok || api.retry() !== api.retry()) throw new Error('retry cache');
console.log(JSON.stringify(events));
"#
        ),
        b"[\"api\",\"lazy\",\"a\",\"b\",\"retry\",\"retry\"]\n"
    );
}

fn assert_browser_transport(artifacts: &Artifacts) {
    for (path, text) in &artifacts.files {
        assert_ne!(path.extension().and_then(|ext| ext.to_str()), Some("cjs"));
        assert!(!text.contains("node:"), "{}: {text}", path.display());
        assert!(!text.contains("require("), "{}: {text}", path.display());
    }
}

#[test]
fn browser_commonjs_entry_aliases_snapshots_and_target_switches() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "entry.cjs",
        r#"
globalThis.count = (globalThis.count || 0) + 1;
exports.value = 1;
exports.bump = () => { module.exports.value++; };
exports.paths = [__filename, __dirname];
"#,
    );
    write(
        dir.root(),
        "main.mjs",
        r#"
import value, {value as snapshot} from './entry.cjs';
import * as namespace from './entry.cjs';
export {value, snapshot, namespace};
export const load = () => import('./entry.cjs');
"#,
    );
    let mut config = browser_options();
    config.entries.insert(
        "cjs".into(),
        Entry {
            path: "entry.cjs".into(),
            output_name: None,
            namespace: true,
        },
    );
    config.entries.insert(
        "alias".into(),
        Entry {
            path: "entry.cjs".into(),
            output_name: Some("nested/alias.mjs".into()),
            namespace: true,
        },
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&config).unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        result.product.artifacts.entries["cjs"],
        PathBuf::from("entry.mjs")
    );
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
const main = await import('./main.mjs');
const direct = await import('./entry.mjs');
const alias = await import('./nested/alias.mjs');
if (main.value !== direct.default || direct.default !== alias.default || globalThis.count !== 1) throw new Error('entry identity');
main.value.bump();
const dynamic = await main.load();
if (main.snapshot !== 1 || direct.value !== 1 || alias.value !== 1) throw new Error('snapshots');
if (main.namespace.value !== 2 || dynamic.value !== 2 || dynamic.default !== main.value) throw new Error('namespace');
if (!main.value.paths[0].endsWith('.mjs') || !main.value.paths[1].endsWith('/native/')) throw new Error('output URLs');
console.log('entries');
"#
        ),
        b"entries\n"
    );
    let fresh = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(fresh.product.artifacts, result.product.artifacts);
    config.entries.remove("alias");
    config.resolver.target_env = TargetEnv::Node;
    assert!(session
        .build(&config)
        .unwrap()
        .product
        .artifacts
        .files
        .keys()
        .any(|path| path.extension().is_some_and(|ext| ext == "cjs")));
    config.resolver.target_env = TargetEnv::Browser;
    assert_browser_transport(&session.build(&config).unwrap().product.artifacts);
}

#[test]
fn browser_commonjs_shared_leaf_imports_are_deduplicated_per_availability_chunk() {
    for (leaf, source, executions) in [
        ("leaf.mjs", "export const value = 1;", 0),
        (
            "leaf.mjs",
            "globalThis.executions++; export const value = 1;",
            1,
        ),
        ("leaf.json", r#"{"value":1}"#, 0),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.cjs",
            &format!("exports.one = require('./{leaf}'); exports.two = require('./child.cjs');"),
        );
        write(
            dir.root(),
            "child.cjs",
            &format!("module.exports = require('./{leaf}');"),
        );
        write(dir.root(), leaf, source);
        let mut config = browser_options();
        config.entries.get_mut("main").unwrap().path = "main.cjs".into();
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_browser_transport(&result.product.artifacts);
        assert_eq!(
            observe_named(
                &result.product.artifacts,
                &format!(
                    r#"
globalThis.executions = 0;
const {{default: api}} = await import('./main.mjs');
if (api.one !== api.two || api.one.value !== 1) throw Error('shared identity');
if (executions !== {executions}) throw Error('execution count');
if ((await import('./main.mjs')).default !== api) throw Error('entry cache');
console.log('shared leaf');
"#
                )
            ),
            b"shared leaf\n"
        );
    }
}

#[test]
fn browser_builtin_aliases_bundle_explicit_stubs_and_track_changes() {
    for (stub, source) in [
        ("stub.cjs", "module.exports = require('./value.cjs');"),
        ("stub.mjs", "export const value = 7;"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "export {default} from './api.cjs';");
        write(
            dir.root(),
            "api.cjs",
            "exports.one = require('fs'); exports.two = require('node:fs');",
        );
        write(dir.root(), stub, source);
        write(dir.root(), "value.cjs", "exports.value = 7;");
        let mut config = browser_options();
        for request in ["fs", "node:fs"] {
            config
                .resolver
                .alias
                .insert(request.into(), format!("./{stub}"));
            // An explicit local replacement wins over an external declaration.
            config.externals.insert(request.into());
        }
        let mut session = BuildSession::new(dir.root()).unwrap();
        for value in [7, 8] {
            if value == 8 {
                if stub == "stub.cjs" {
                    write(dir.root(), "value.cjs", "exports.value = 8;");
                } else {
                    write(dir.root(), stub, "export const value = 8;");
                }
            }
            let result = session.build(&config).unwrap();
            assert_browser_transport(&result.product.artifacts);
            assert_eq!(
                observe_named(
                    &result.product.artifacts,
                    &format!(
                        "const {{default: api}} = await import('./main.mjs'); if (api.one !== api.two || api.one.value !== {value}) throw Error('stub identity/value'); console.log('stub');"
                    )
                ),
                b"stub\n"
            );
        }
    }
}

#[test]
fn browser_require_aliases_do_not_hide_invalid_targets() {
    for (request, replacement, external, diagnostic) in [
        ("fs", None, false, "Node builtin"),
        ("node:fs", None, true, "Node builtin"),
        ("missing", None, false, "missing"),
        ("missing", Some("./absent.cjs"), false, "absent.cjs"),
        ("fs", Some("./absent.cjs"), false, "absent.cjs"),
        ("fs", Some("node:fs"), false, "Node builtin"),
        ("shim", Some("fs"), false, "Node builtin"),
        ("fs", Some("./addon.node"), false, "native addon"),
        (
            "fs",
            Some("./.bundler-hidden/stub.cjs"),
            false,
            "outside reachable build inventory",
        ),
        ("external-pkg", None, true, "cannot require external"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "export {default} from './api.cjs';");
        write(
            dir.root(),
            "api.cjs",
            &format!("module.exports = require('{request}');"),
        );
        write(dir.root(), "addon.node", "");
        write(dir.root(), ".bundler-hidden/stub.cjs", "exports.value = 1;");
        let mut config = browser_options();
        if let Some(replacement) = replacement {
            config
                .resolver
                .alias
                .insert(request.into(), replacement.into());
        }
        if external {
            config.externals.insert(request.into());
        }
        let error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("api.cjs") && error.contains(request) && error.contains(diagnostic),
            "{error}"
        );
    }
}

#[test]
fn browser_require_esm_leaf_preserves_timing_live_bindings_and_cached_errors() {
    for esm in [false, true] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            if esm {
                "const load = () => require('./leaf.mjs'); const fail = () => require('./fail.mjs'); export default {load, fail};"
            } else {
                "export {default} from './api.cjs';"
            },
        );
        write(
            dir.root(),
            "api.cjs",
            r#"
exports.load = () => require('./leaf.mjs');
exports.fail = () => require('./fail.mjs');
"#,
        );
        write(
            dir.root(),
            "leaf.mjs",
            r#"
events.push('leaf');
export let value = 1;
export const bump = () => value++;
export {value as alias};
export default function () { return value; }
export const topThis = this;
"#,
        );
        write(
            dir.root(),
            "fail.mjs",
            "events.push('fail'); throw new Error('leaf failure');",
        );
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&browser_options())
            .unwrap();
        assert_browser_transport(&result.product.artifacts);
        assert_eq!(
            observe_named(
                &result.product.artifacts,
                r#"
globalThis.events = [];
const {default: api} = await import('./main.mjs');
if (events.length) throw new Error('eager ESM');
events.push('before');
const ns = api.load();
events.push('after');
ns.bump();
if (ns.value !== 2 || ns.alias !== 2 || ns.default() !== 2 || ns.default.name !== 'default' || ns.topThis !== undefined || api.load() !== ns) throw new Error('live exports');
try { ns.value = 42; throw new Error('writable'); } catch (e) { if (!(e instanceof TypeError)) throw e; }
let first;
try { api.fail(); } catch(e) { first = e; }
try { api.fail(); } catch(e) { if (e !== first) throw new Error('ESM error identity'); }
console.log(JSON.stringify(events));
"#
            ),
            b"[\"before\",\"leaf\",\"after\",\"fail\"]\n"
        );
    }
}

#[test]
fn browser_require_rejects_external_async_and_shared_native_esm_boundaries() {
    for esm in [false, true] {
        for (entry, source, extra, diagnostic) in [
            (
                "export {default} from './api.cjs';",
                "exports.load = () => require('external-pkg');",
                "",
                "cannot require external",
            ),
            (
                "export {default} from './api.cjs';",
                "exports.load = () => require('./leaf.mjs');",
                "await 0; export const value = 1;",
                "async",
            ),
            (
                "import './leaf.mjs'; export {default} from './api.cjs';",
                "exports.load = () => require('./leaf.mjs');",
                "console.log('leaf'); export let value = 1;",
                "native static/dynamic",
            ),
            (
                "export {default} from './api.cjs';",
                "exports.load = () => require('./leaf.mjs');",
                "import './other.mjs'; export let value = 1;",
                "dependency-free leaf",
            ),
            (
                "export const dynamic = () => import('./leaf.mjs'); export {default} from './api.cjs';",
                "exports.load = () => require('./leaf.mjs');",
                "console.log('leaf'); export const value = 1;",
                "native entry or dynamic-import job",
            ),
        ] {
            let dir = TmpDir::new();
            let entry = if esm {
                entry.replace("api.cjs", "api.mjs")
            } else {
                entry.into()
            };
            let source = if esm {
                format!(
                    "{} export default {{load}};",
                    source.replace("exports.load =", "export const load =")
                )
            } else {
                source.into()
            };
            write(dir.root(), "main.mjs", &entry);
            write(dir.root(), if esm { "api.mjs" } else { "api.cjs" }, &source);
            write(dir.root(), "leaf.mjs", extra);
            write(dir.root(), "other.mjs", "export {};");
            let mut config = browser_options();
            config.externals.insert("external-pkg".into());
            let error = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .expect("unsupported boundary");
            assert!(format!("{error:#}").contains(diagnostic), "{error:#}");
        }
    }
}

#[test]
fn browser_require_in_esm_json_ordinary_and_reconstructed_share_default_identity() {
    for ordinary in [false, true] {
        let dir = TmpDir::new();
        let external = "data:text/javascript,export{}";
        let mut config = browser_options();
        config.output.min_chunk_bytes = 0;
        let prefix = if ordinary {
            config.externals.insert(external.into());
            format!("import '{external}';")
        } else {
            String::new()
        };
        write(
            dir.root(),
            "main.mjs",
            &format!(
                "{prefix}
            export * from './lib/runtimeCreator.js';
            import data from './package.json';
            import * as ns from './leaf.mjs';
            import cjsLoad from './loader.cjs';
            export {{data, ns, cjsLoad}};
            export const dynamic = () => import('./package.json');
            export const leaf = () => require('./leaf.mjs');
        "
            ),
        );
        write(dir.root(), "lib/runtimeCreator.js",
            "const rawChatLoaderVersion = require('../package.json').version; export const chatLoaderVersion = String(rawChatLoaderVersion); export const load = () => (require)(('../package.json'));");
        write(
            dir.root(),
            "package.json",
            r#"{"version":"26.0.114","nested":{"value":1}}"#,
        );
        write(
            dir.root(),
            "loader.cjs",
            "module.exports = () => require('./package.json');",
        );
        write(
            dir.root(),
            "leaf.mjs",
            "export let value = 1; export const bump = () => value++;",
        );
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&config).unwrap();
        assert_browser_transport(&result.product.artifacts);
        assert_eq!(result.counters.source_reparses, 0);
        assert_eq!(result.counters.fragment_reparses, 0);
        let retains_native_export = result
            .product
            .artifacts
            .files
            .values()
            .any(|source| source.contains("export const chatLoaderVersion"));
        assert_eq!(retains_native_export, ordinary);
        assert_eq!(
            observe_named(
                &result.product.artifacts,
                r#"
const entry = await import('./main.mjs');
if (entry.chatLoaderVersion !== '26.0.114') throw Error('version');
if (entry.data !== entry.load() || entry.data !== entry.cjsLoad() ||
    entry.data !== (await entry.dynamic()).default) throw Error('JSON identity');
entry.data.nested.value = 7;
if (entry.load().nested.value !== 7) throw Error('JSON mutation');
if (entry.leaf() !== entry.ns) throw Error('namespace identity');
entry.ns.bump();
if (entry.leaf().value !== 2) throw Error('live namespace');
console.log(entry.chatLoaderVersion);
"#
            ),
            b"26.0.114\n"
        );
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&result.product, &noop.product));
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.product.artifacts, result.product.artifacts);
    }
}

#[test]
fn browser_require_in_esm_cjs_ordinary_and_reconstructed_defer_cache_and_retry() {
    for target in [TargetEnv::Browser, TargetEnv::Node] {
        for ordinary in [false, true] {
            let dir = TmpDir::new();
            let mut config = browser_options();
            config.resolver.target_env = target;
            let external = "data:text/javascript,export{}";
            let prefix = if ordinary {
                config.externals.insert(external.into());
                format!("import '{external}';")
            } else {
                String::new()
            };
            write(
                dir.root(),
                "main.mjs",
                &format!(
                    "{prefix}
            events.push('entry');
            export const load = () => require('./value.cjs');
            export const again = () => (require)(('./value.cjs'));
            export const retry = () => require('./retry.cjs');
            export const conditional = flag => flag ? require('./value.cjs') : null;
            export const dynamic = () => import('./value.cjs');
        "
                ),
            );
            write(
                dir.root(),
                "value.cjs",
                "events.push('value'); module.exports = {value: 7};",
            );
            write(dir.root(), "retry.cjs", "events.push('retry'); if (!globalThis.attempt++) throw Error('first'); module.exports = {ok: true};");
            let result = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            if target == TargetEnv::Browser {
                assert_browser_transport(&result.product.artifacts);
            }
            assert_eq!(
                observe_named(
                    &result.product.artifacts,
                    r#"
globalThis.events = []; globalThis.attempt = 0;
const api = await import('./main.mjs');
if (events.join() !== 'entry' || api.conditional(false) !== null) throw Error('eager');
events.push('before');
const value = api.load(); events.push('after');
if (value !== api.again() || value !== api.conditional(true) || value.value !== 7) throw Error('cache');
if ((await api.dynamic()).default !== value) throw Error('interop cache');
try { api.retry(); throw Error('missing throw'); } catch (e) { if (e.message !== 'first') throw e; }
if (!api.retry().ok || api.retry() !== api.retry()) throw Error('retry cache');
console.log(JSON.stringify(events));
"#
                ),
                b"[\"entry\",\"before\",\"value\",\"after\",\"retry\",\"retry\"]\n"
            );
        }
    }
}

#[test]
fn browser_require_pure_esm_leaf_shares_native_live_namespace() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './api.cjs';
import * as ns from './leaf.mjs';
import json from './data.json';
import asset from './image.svg';
import style from './style.css';
export {api, ns, json, asset, style};
export const load = () => import('./leaf.mjs');
"#,
    );
    write(
        dir.root(),
        "api.cjs",
        r#"
exports.load = () => require('./leaf.mjs');
exports.json = () => require('./data.json');
exports.asset = () => require('./image.svg');
exports.style = () => require('./style.css');
"#,
    );
    write(
        dir.root(),
        "leaf.mjs",
        "export let value = 1; export const bump = () => value++;",
    );
    write(dir.root(), "data.json", r#"{"nested":{"value":1}}"#);
    write(dir.root(), "image.svg", "<svg/>");
    write(dir.root(), "style.css", "body { color: red }");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&browser_options())
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
const {api, ns, json, asset, style, load} = await import('./main.mjs');
if (ns !== api.load() || ns !== await load()) throw new Error('native namespace identity');
ns.bump();
if (api.load().value !== 2) throw new Error('live binding');
if (api.json() !== json) throw new Error('JSON identity');
json.nested.value = 7;
if (api.json().nested.value !== 7) throw new Error('JSON mutation');
if (api.asset() !== asset || api.style() !== style) throw new Error('resource defaults');
console.log('shared');
"#
        ),
        b"shared\n"
    );
}

#[test]
fn browser_commonjs_late_facades_use_captured_intrinsics() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './api.cjs';
export {api};
export const load = () => import('./late.cjs');
"#,
    );
    write(
        dir.root(),
        "api.cjs",
        "exports.load = () => require('./late.cjs');",
    );
    write(dir.root(), "late.cjs", "exports.value = 7;");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&browser_options())
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
const entry = await import('./main.mjs');
const originals = [Object.create, Object.defineProperty, Object.getOwnPropertyNames, Object.freeze, Object.assign, Reflect.apply];
const fail = () => { throw new Error('uncaptured intrinsic'); };
Object.create = Object.defineProperty = Object.getOwnPropertyNames = Object.freeze = Object.assign = Reflect.apply = fail;
const value = entry.api.load();
const ns = await entry.load();
[Object.create, Object.defineProperty, Object.getOwnPropertyNames, Object.freeze, Object.assign, Reflect.apply] = originals;
if (ns.value !== 7 || ns.default !== value) throw new Error('identity');
console.log('intrinsics');
"#
        ),
        b"intrinsics\n"
    );
}

#[test]
fn browser_commonjs_failed_dynamic_job_stays_failed_while_require_retries() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './api.cjs';
export {api};
export const load = () => import('./retry.cjs');
"#,
    );
    write(
        dir.root(),
        "api.cjs",
        "exports.load = () => require('./retry.cjs');",
    );
    write(
        dir.root(),
        "retry.cjs",
        r#"
globalThis.attempts = (globalThis.attempts || 0) + 1;
if (globalThis.attempts === 1) throw new Error('first');
exports.value = 7;
"#,
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&browser_options())
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
const entry = await import('./main.mjs');
let first;
try { await entry.load(); } catch (error) { first = error; }
if (!first || first.message !== 'first') throw new Error('dynamic failure');
if (entry.api.load().value !== 7 || globalThis.attempts !== 2) throw new Error('require retry');
try { await entry.load(); throw new Error('unexpected retry'); }
catch (error) { if (error !== first) throw error; }
console.log('cached failure');
"#
        ),
        b"cached failure\n"
    );
}

#[test]
fn browser_require_css_auto_mount_keeps_synchronous_activation_and_url_relocation() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export {default} from './api.cjs';");
    write(
        dir.root(),
        "api.cjs",
        "exports.load = () => require('./style.css');",
    );
    write(dir.root(), "style.css", "body { color: red }");
    let mut config = browser_options();
    config.css.load = CssLoad::AutoMount;
    config.entries.get_mut("main").unwrap().output_name = Some("nested/main.mjs".into());
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    assert_eq!(
        observe_named(
            &result.product.artifacts,
            r#"
const events = [];
globalThis.document = {
  createElement() { return {}; },
  head: {appendChild(link) {
    if (!link.href.startsWith(new URL('./assets/', import.meta.url).href)) throw new Error('URL');
    events.push(link.href);
  }}
};
const {default: api} = await import('./nested/main.mjs');
if (events.length) throw new Error('eager CSS');
const ns = api.load();
if (events.length !== 1 || ns.default !== events[0] || api.load() !== ns) throw new Error('CSS activation');
console.log('CSS');
"#
        ),
        b"CSS\n"
    );
}

/// Optional real-engine check: no browser download and no Node loader in the page.
#[test]
fn browser_commonjs_runs_in_firefox() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let Some(browser) = std::env::var_os("BUNDLER_TEST_FIREFOX") else {
        eprintln!("BUNDLER_TEST_FIREFOX is unset; real-browser check skipped");
        return;
    };
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
import api from './api.cjs';
export {api};
export const dynamic = () => import('./lazy.cjs');
"#,
    );
    write(
        dir.root(),
        "api.cjs",
        r#"
exports.load = () => require('./lazy.cjs');
exports.context = name => require(`./parts/${name()}.cjs`);
exports.leaf = () => require('./leaf.mjs');
exports.pure = () => require('./pure.mjs');
exports.json = () => require('./data.json');
exports.stub = () => require('fs');
exports.esmStub = () => require('node:fs');
exports.child = () => require('./child.cjs');
exports.cycle = () => require('./a.cjs');
exports.retry = () => require('./retry.cjs');
"#,
    );
    write(
        dir.root(),
        "lazy.cjs",
        "events.push('lazy'); module.exports = {value: 7};",
    );
    write(
        dir.root(),
        "parts/a.cjs",
        "events.push('context'); exports.value = 8;",
    );
    write(
        dir.root(),
        "a.cjs",
        "exports.early = 1; exports.b = require('./b.cjs'); exports.done = 2;",
    );
    write(dir.root(), "b.cjs", "exports.early = require('./a.cjs').early; exports.partial = require('./a.cjs').done === undefined;");
    write(
        dir.root(),
        "retry.cjs",
        "if (!globalThis.attempts++) throw new Error('retry'); exports.ok = true;",
    );
    write(
        dir.root(),
        "leaf.mjs",
        "events.push('leaf'); export let value = 1; export const bump = () => value++;",
    );
    write(dir.root(), "pure.mjs", "export const value = 1;");
    write(dir.root(), "data.json", r#"{"value":1}"#);
    write(
        dir.root(),
        "stub.cjs",
        "events.push('stub'); module.exports = require('./stub-value.cjs');",
    );
    write(dir.root(), "stub-value.cjs", "exports.value = 9;");
    write(dir.root(), "stub.mjs", "export const value = 10;");
    write(
        dir.root(),
        "child.cjs",
        r#"
exports.leaf = require('./leaf.mjs');
exports.pure = require('./pure.mjs');
exports.json = require('./data.json');
exports.stub = require('fs');
exports.esmStub = require('node:fs');
"#,
    );
    let mut config = browser_options();
    config
        .resolver
        .alias
        .insert("fs".into(), "./stub.cjs".into());
    config
        .resolver
        .alias
        .insert("node:fs".into(), "./stub.mjs".into());
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_browser_transport(&result.product.artifacts);
    let html = r#"<!doctype html><script type="module">
try {
  if (typeof require !== 'undefined' || typeof module !== 'undefined') throw Error('host loader');
  globalThis.events = []; globalThis.attempts = 0;
  const entry = await import('/main.mjs');
  if (events.length) throw Error('eager');
  const value = entry.api.load();
  if (value.value !== 7 || (await entry.dynamic()).default !== value) throw Error('identity');
  let calls = 0;
  if (entry.api.context(() => { calls++; return 'a'; }).value !== 8 || calls !== 1) throw Error('context');
  try { entry.api.context(() => 'missing'); throw Error('miss'); }
  catch (e) { if (e.code !== 'MODULE_NOT_FOUND') throw e; }
  const cycle = entry.api.cycle();
  if (cycle.b.early !== 1 || !cycle.b.partial || cycle.done !== 2) throw Error('cycle');
  try { entry.api.retry(); throw Error('unexpected'); } catch (e) { if (e.message !== 'retry') throw e; }
  if (!entry.api.retry().ok || attempts !== 2) throw Error('retry cache');
  const leaf = entry.api.leaf(); leaf.bump();
  if (leaf.value !== 2 || leaf !== entry.api.leaf()) throw Error('live ESM');
  const pure = entry.api.pure(), json = entry.api.json(), stub = entry.api.stub(), esmStub = entry.api.esmStub();
  const child = entry.api.child();
  if (child.leaf !== leaf || child.leaf.value !== 2 || child.pure !== pure || pure.value !== 1) throw Error('shared ESM');
  if (child.json !== json || json.value !== 1) throw Error('shared JSON');
  json.value = 2;
  if (child.json.value !== 2) throw Error('JSON mutation');
  if (child.stub !== stub || stub.value !== 9 || child.esmStub !== esmStub || esmStub.value !== 10) throw Error('explicit stubs');
  if (events.join() !== 'lazy,context,leaf,stub') throw Error('timing/once');
  await fetch('/result?ok');
} catch (error) { await fetch('/result?' + encodeURIComponent(error.stack || error)); }
</script>"#;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let profile = TmpDir::new();
    let mut child = Command::new(browser)
        .args(["--headless", "--no-remote", "--profile"])
        .arg(profile.root())
        .arg(format!("http://{}/", listener.local_addr().unwrap()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut report = None;
    while Instant::now() < deadline && child.try_wait().unwrap().is_none() {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 8192];
        let Ok(count) = stream.read(&mut request) else {
            continue;
        };
        let request = String::from_utf8_lossy(&request[..count]);
        let path = request.split_whitespace().nth(1).unwrap_or("/");
        let (mime, body) = if path == "/" {
            ("text/html", html)
        } else if let Some(message) = path.strip_prefix("/result?") {
            report = Some(message.to_owned());
            ("text/plain", "received")
        } else {
            (
                "text/javascript",
                result
                    .product
                    .artifacts
                    .files
                    .get(Path::new(path.trim_start_matches('/')))
                    .map(String::as_str)
                    .unwrap_or(""),
            )
        };
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        if report.is_some() {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        report.as_deref(),
        Some("ok"),
        "Firefox did not confirm browser execution: {report:?}"
    );
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
    config.loaders = crate::loaders::LoaderPolicy::default();
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
