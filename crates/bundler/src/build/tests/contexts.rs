use super::*;

#[test]
fn legacy_commonjs_contexts_keep_parent_requests_and_partial_prefix_matching() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {default as load} from './loaders/loader.cjs';",
    );
    write(
        dir.root(),
        "loaders/loader.cjs",
        "module.exports = name => require('../parts/pre' + name + '.js'); module.exports.local = name => require('./pre' + name + '.js');",
    );
    write(dir.root(), "parts/prefix.js", "exports.value = 1;");
    write(dir.root(), "parts/pre/nested.js", "exports.value = 2;");
    write(dir.root(), "loaders/pre/nested.js", "exports.value = 3;");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(
        observe_named(&result.product.artifacts, "const entry = await import('./main.mjs'); console.log(entry.load('fix').value, entry.load('/nested').value, entry.load.local('/nested').value);"),
        b"1 2 3\n"
    );
}

#[test]
fn esm_require_context_component_terminal_and_multiple_gaps_are_lazy_and_exact() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        r#"
export {partial, partialTerminal} from './factory.js';
export const terminal = name => require('./parts/' + name);
export const multiple = (a, b) => require(`./parts/${a}-${b}.cjs`);
export const literal = name => require('./pre[.+]fix' + name + '.cjs');
"#,
    );
    write(
        dir.root(),
        "factory.js",
        "export const partial = name => require('./prefix' + name() + '.js'); export const partialTerminal = name => require('./prefix' + name);",
    );
    write(
        dir.root(),
        "prefixone.js",
        "globalThis.events.push('partial'); exports.value = 1;",
    );
    write(dir.root(), "prefix/deep.js", "this is not valid JavaScript");
    write(
        dir.root(),
        "parts/deep/esm.mjs",
        "globalThis.events.push('esm'); export const value = 2;",
    );
    write(
        dir.root(),
        "parts/dir/index.cjs",
        "globalThis.events.push('index'); exports.value = 3;",
    );
    write(
        dir.root(),
        "parts/nested/a-b.cjs",
        "globalThis.events.push('multiple'); exports.value = 4;",
    );
    write(
        dir.root(),
        "pre[.+]fixone.cjs",
        "globalThis.events.push('literal'); exports.value = 5;",
    );
    write(dir.root(), "preZfixone.cjs", "this is not valid JavaScript");
    let observer = r#"
globalThis.events = [];
const entry = await import('./main.mjs');
if (events.length) throw Error('eager context activation');
let calls = 0;
const partial = entry.partial(() => { calls++; return 'one'; });
if (calls !== 1 || partial.value !== 1 || partial !== entry.partial(() => 'one')) throw Error('partial/once');
if (entry.partialTerminal('one.js') !== partial) throw Error('terminal component identity');
const esm = entry.terminal('deep/esm.mjs');
if (esm.value !== 2 || esm !== entry.terminal('deep/esm')) throw Error('ESM alias/identity');
const index = entry.terminal('dir');
if (index.value !== 3 || index !== entry.terminal('dir/') || index !== entry.terminal('dir/index.cjs')) throw Error('index aliases');
if (entry.multiple('nested/a', 'b').value !== 4 || entry.literal('one').value !== 5) throw Error('multi/literal');
for (const invoke of [
  () => entry.partial(() => '/deep'),
  () => entry.partialTerminal('/deep.js'),
  () => entry.multiple('nested', 'a/b'),
  () => entry.terminal('missing'),
  () => entry.terminal('../prefixone.js'),
  () => entry.terminal('deep/../deep/esm.mjs'),
  () => entry.terminal('..\\prefixone.js'),
  () => entry.terminal('/absolute'),
  () => entry.terminal('node:fs'),
  () => entry.terminal('__proto__'),
]) {
  try { invoke(); throw Error('unexpected resolution'); }
  catch (error) { if (error.code !== 'MODULE_NOT_FOUND') throw error; }
}
console.log(JSON.stringify(events));
"#;
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    config.resolver.extensions = vec!["js".into(), "mjs".into(), "cjs".into()];
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(
        observe_named(&cold.product.artifacts, observer),
        b"[\"partial\",\"esm\",\"index\",\"multiple\",\"literal\"]\n"
    );
    assert!(Rc::ptr_eq(
        &cold.product,
        &session.build(&config).unwrap().product
    ));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(cold.product.artifacts, disk.product.artifacts);
    assert_eq!(
        observe_named(&disk.product.artifacts, observer),
        b"[\"partial\",\"esm\",\"index\",\"multiple\",\"literal\"]\n"
    );
    config.cache = CacheMode::Disabled;
    let disabled = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(cold.product.artifacts, disabled.product.artifacts);
    assert_eq!(
        observe_named(&disabled.product.artifacts, observer),
        b"[\"partial\",\"esm\",\"index\",\"multiple\",\"literal\"]\n"
    );
}

#[test]
fn esm_require_context_new_gap_inventory_changes_preserve_cache_parity() {
    for (expression, prefix, suffix) in [
        ("'./parts/pre' + name + '.cjs'", "pre", ".cjs"),
        ("'./parts/' + name", "", ".cjs"),
        ("`./parts/${name}-fixed${tail}.cjs`", "", "-fixed.cjs"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", &format!("export {{sep}} from 'node:path'; export const load = (name, tail = '') => require({expression});"));
        let candidate = |name| format!("parts/{prefix}{name}{suffix}");
        let request = |name: &str| {
            if expression == "'./parts/' + name" {
                format!("{name}.cjs")
            } else {
                name.to_owned()
            }
        };
        write(dir.root(), &candidate("a"), "exports.value = 1;");
        let mut config = options();
        config.externals.insert("node:path".into());
        let mut session = BuildSession::new(dir.root()).unwrap();
        let first = session.build(&config).unwrap();
        write(dir.root(), "unrelated.cjs", "this is not valid JavaScript");
        assert!(Rc::ptr_eq(
            &first.product,
            &session.build(&config).unwrap().product
        ));
        if prefix == "pre" {
            write(
                dir.root(),
                "parts/pre/deep.cjs",
                "this is not valid JavaScript",
            );
            assert!(Rc::ptr_eq(
                &first.product,
                &session.build(&config).unwrap().product
            ));
        }
        let mut previous = first.product;
        for action in ["add", "rename", "remove"] {
            match action {
                "add" => write(dir.root(), &candidate("b"), "exports.value = 2;"),
                "rename" => fs::rename(
                    dir.root().join(candidate("b")),
                    dir.root().join(candidate("c")),
                )
                .unwrap(),
                _ => fs::remove_file(dir.root().join(candidate("c"))).unwrap(),
            }
            let memory = session.build(&config).unwrap();
            assert!(!Rc::ptr_eq(&previous, &memory.product));
            previous = memory.product.clone();
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            let mut disabled = config.clone();
            disabled.cache = CacheMode::Disabled;
            let fresh = BuildSession::new(dir.root())
                .unwrap()
                .build(&disabled)
                .unwrap();
            assert_eq!(
                memory.product.artifacts, disk.product.artifacts,
                "{expression}: {action}"
            );
            assert_eq!(
                memory.product.artifacts, fresh.product.artifacts,
                "{expression}: {action}"
            );
            let present = if action == "add" {
                "b"
            } else if action == "rename" {
                "c"
            } else {
                "a"
            };
            assert_eq!(observe_named(&memory.product.artifacts, &format!(
                "const entry = await import('./main.mjs'); console.log(entry.load({:?}).value);", request(present)
            )), if action == "remove" { &b"1\n"[..] } else { &b"2\n"[..] });
        }
    }
}

#[test]
fn esm_require_context_factory_is_synchronous_lazy_and_cache_independent() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(
        dir.root(),
        "enums.mjs",
        "export const HostClientType = 'cjs';",
    );
    write(
        dir.root(),
        "main.mjs",
        r#"
import {HostClientType} from './enums.mjs';
export const initial = HostClientType;
export const factory = CLIENT => new (require(`./clients/${CLIENT}/HostClient`).default)();
export const concat = name => require('./clients/' + name() + '/HostClient');
export const nested = name => require(`./clients/${require('./name.cjs')(name)}/HostClient`);
export const shadow = (require, name) => require(`./clients/${name}/HostClient`);
export function ordered(tail) { return require(`./clients/${this.pick()}${arguments[0]()}/HostClient`); }
"#,
    );
    write(dir.root(), "name.cjs", "module.exports = name => name;");
    write(
        dir.root(),
        "clients/cjs/package.json",
        r#"{"type":"commonjs"}"#,
    );
    write(
        dir.root(),
        "clients/unused/package.json",
        r#"{"type":"commonjs"}"#,
    );
    write(
        dir.root(),
        "clients/cjs/HostClient.js",
        "globalThis.events.push('cjs'); exports.default = class { value = 'cjs'; };",
    );
    write(
        dir.root(),
        "clients/esm/HostClient.js",
        "globalThis.events.push('esm'); export default class { value = 'esm'; }; export const named = 7;",
    );
    write(
        dir.root(),
        "clients/unused/HostClient.js",
        "globalThis.events.push('unused'); exports.default = class {};",
    );
    let observer = r#"
globalThis.events = [];
const entry = await import('./main.mjs');
if (events.length) throw Error('eager context activation');
let calls = 0;
const ns = entry.concat(() => { calls++; events.push('argument'); return entry.initial; });
if (calls !== 1 || ns !== entry.nested('cjs') || entry.factory('cjs').value !== 'cjs') throw Error('CJS identity');
if (entry.factory('esm').value !== 'esm') throw Error('ESM default');
const esm = entry.concat(() => 'esm');
if (esm.named !== 7 || esm !== entry.nested('esm')) throw Error('ESM namespace');
if (entry.shadow(x => x, 'shadow') !== './clients/shadow/HostClient') throw Error('shadow');
const order = [];
const ordered = entry.ordered.call({pick() { order.push('first'); return 'cj'; }}, () => { order.push('second'); return 's'; });
if (ordered !== ns || order.join() !== 'first,second') throw Error('argument context/order');
const failure = {};
try {
  entry.ordered.call({pick() { return {toString() { throw failure; }}; }}, () => { throw Error('evaluated later argument'); });
  throw Error('expected coercion failure');
} catch (error) { if (error !== failure) throw error; }
for (const name of ['missing', '../cjs', 'cjs/../cjs', '..\\cjs', '__proto__', 'toString', '...']) {
  try { entry.factory(name); throw Error('unexpected resolution'); }
  catch (error) {
    if (error.code !== 'MODULE_NOT_FOUND') throw error;
    if (name !== '...' && name.includes('..') && !error.message.includes('Parent traversal')) throw error;
    if (name === '...' && error.message.includes('Parent traversal')) throw error;
    if (name === 'missing' && error.message !== 'Module not found: ./clients/missing/HostClient') throw error;
  }
}
console.log(JSON.stringify(events));
"#;
    {
        let mut config = options();
        config.resolver.target_env = TargetEnv::Browser;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert_eq!(
            observe_named(&cold.product.artifacts, observer),
            b"[\"argument\",\"cjs\",\"esm\"]\n"
        );
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(disk.counters.source_parses, 0);
        assert_eq!(cold.product.artifacts, disk.product.artifacts);
        assert_eq!(
            observe_named(&disk.product.artifacts, observer),
            b"[\"argument\",\"cjs\",\"esm\"]\n"
        );
        config.cache = CacheMode::Disabled;
        let disabled = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(cold.product.artifacts, disabled.product.artifacts);
        assert_eq!(
            observe_named(&disabled.product.artifacts, observer),
            b"[\"argument\",\"cjs\",\"esm\"]\n"
        );
        assert_eq!(cold.counters.source_reparses, 0);
        assert_eq!(cold.counters.fragment_reparses, 0);
    }
}

#[test]
fn esm_require_context_inventory_changes_invalidate_memory_disk_and_disabled_builds() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export {sep} from 'node:path'; export const load = name => require(`./parts/${name}/HostClient`);",
    );
    write(dir.root(), "parts/a/HostClient.js", "exports.default = 1;");
    let mut config = options();
    config.externals.insert("node:path".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let initial = session.build(&config).unwrap();
    write(dir.root(), "parts/unrelated.txt", "not a candidate");
    assert!(Rc::ptr_eq(
        &initial.product,
        &session.build(&config).unwrap().product
    ));
    let mut previous = initial.product.clone();
    for change in ["add", "rename", "remove"] {
        match change {
            "add" => write(dir.root(), "parts/b/HostClient.js", "exports.default = 2;"),
            "rename" => {
                fs::create_dir_all(dir.root().join("parts/c")).unwrap();
                fs::rename(
                    dir.root().join("parts/b/HostClient.js"),
                    dir.root().join("parts/c/HostClient.js"),
                )
                .unwrap();
            }
            _ => fs::remove_file(dir.root().join("parts/c/HostClient.js")).unwrap(),
        }
        let memory = session.build(&config).unwrap();
        assert!(!Rc::ptr_eq(&previous, &memory.product));
        previous = memory.product.clone();
        assert_eq!(memory.counters.source_reparses, 0);
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(memory.product.artifacts, disk.product.artifacts, "{change}");
        let mut disabled = config.clone();
        disabled.cache = CacheMode::Disabled;
        let fresh = BuildSession::new(dir.root())
            .unwrap()
            .build(&disabled)
            .unwrap();
        assert_eq!(
            memory.product.artifacts, fresh.product.artifacts,
            "{change}"
        );
        let present = match change {
            "add" => "b",
            "rename" => "c",
            _ => "a",
        };
        assert_eq!(
            observe_named(
                &memory.product.artifacts,
                &format!(
                    "const entry = await import('./main.mjs'); console.log(entry.load('{present}').default);"
                )
            ),
            if change == "remove" {
                &b"1\n"[..]
            } else {
                &b"2\n"[..]
            }
        );
        let missing = if change == "add" { "c" } else { "b" };
        assert_eq!(
            observe_named(
                &memory.product.artifacts,
                &format!(
                    "const entry = await import('./main.mjs'); try {{ entry.load('{missing}'); }} catch (error) {{ console.log(error.code); }}"
                )
            ),
            b"MODULE_NOT_FOUND\n"
        );
    }
}

#[test]
fn esm_require_context_keeps_existing_async_and_dependent_target_errors() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = name => require(`./parts/${name}.mjs`);",
    );
    write(dir.root(), "dep.mjs", "export const value = 1;");
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    for (source, expected) in [
        ("await 0; export default 1;", "require of async ESM"),
        (
            "import {value} from '../dep.mjs'; export default value;",
            "dependency-free leaf",
        ),
    ] {
        write(dir.root(), "parts/a.mjs", source);
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

#[test]
fn esm_require_context_rechecks_checked_inventory_after_preparation() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = name => require(`./parts/${name}/HostClient`); import './z.css';",
    );
    write(dir.root(), "parts/a/HostClient.js", "exports.default = 1;");
    write(dir.root(), "z.css", "body {}");
    let config = options();
    let root = dir.root().to_owned();
    let observer = observe_preparation(move |path| {
        if path.file_name().is_some_and(|name| name == "z.css") {
            write(&root, "parts/b/HostClient.js", "exports.default = 2;");
        }
    });
    let error = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .err()
        .unwrap();
    drop(observer);
    assert!(
        format!("{error:#}").contains("context candidates changed"),
        "{error:#}"
    );
    let settled = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        observe_named(
            &settled.product.artifacts,
            "const entry = await import('./main.mjs'); console.log(entry.load('b').default);"
        ),
        b"2\n"
    );
}

#[test]
fn runtime_contexts_exclude_declaration_siblings_on_cold_warm_and_noop_builds() {
    for (loader, source) in [
        (
            "loader.mjs",
            // Unsupported concat shapes intentionally retain broad capture.
            r#"export const load = locale => import("../../locales/message_".concat(locale, ".json"));"#,
        ),
        (
            "loader.mjs",
            "export const load = name => import(`./${name}`);",
        ),
        ("loader.cjs", "exports.load = name => require(`./${name}`);"),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!("export {{load}} from './dist/Strings/{loader}';"),
        );
        write(dir.root(), &format!("dist/Strings/{loader}"), source);
        for (file, source) in [
            ("index.js", "export const value = 1;"),
            ("index.ts", "export const typed: number = 2;"),
            ("index.mts", "export const module: number = 3;"),
            ("index.cts", "export = 4;"),
            ("nested/value.mjs", "export default 5;"),
            ("types.d.ts/runtime.mjs", "export default 6;"),
            ("value.d.ts.js", "export default 7;"),
        ] {
            write(dir.root(), &format!("dist/Strings/{file}"), source);
        }
        for extension in ["ts", "mts", "cts"] {
            write(
                dir.root(),
                &format!("dist/Strings/index.d.{extension}"),
                "invalid declaration syntax!",
            );
        }
        let config = options();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        for file in [
            "index.js",
            "index.ts",
            "index.mts",
            "index.cts",
            "nested/value.mjs",
            "types.d.ts/runtime.mjs",
            "value.d.ts.js",
        ] {
            assert!(
                cold.product
                    .graph
                    .file_id(&dir.root().join("dist/Strings").join(file))
                    .is_some(),
                "{loader}: missing {file}"
            );
        }
        assert_eq!(cold.product.graph.file_count(), 9);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.product.artifacts, cold.product.artifacts);
        assert_eq!(warm.warnings, cold.warnings);
        assert_eq!(noop.warnings, cold.warnings);
        assert_eq!(cold.warnings.len(), usize::from(source.contains(".concat")));
    }
}

#[test]
fn runtime_context_declaration_changes_do_not_invalidate_or_fail_final_rechecks() {
    for (loader, source) in [
        ("loader.mjs", "export const load = name => import(name);"),
        (
            "loader.mjs",
            "export const load = name => import(`./${name}`);",
        ),
        ("loader.cjs", "exports.load = name => require(`./${name}`);"),
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            &format!("export {{load}} from './parts/{loader}'; import './z.css';"),
        );
        write(dir.root(), &format!("parts/{loader}"), source);
        write(dir.root(), "parts/value.mjs", "export const value = 1;");
        write(dir.root(), "z.css", "body {}");
        let config = options();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let first = session.build(&config).unwrap();
        for extension in ["ts", "mts", "cts"] {
            write(
                dir.root(),
                &format!("parts/value.d.{extension}"),
                "invalid declaration syntax!",
            );
        }
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&first.product, &noop.product));
        assert_eq!(noop.counters.source_parses, 0);
        let warm = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(warm.counters.source_parses, 0);
        assert_eq!(warm.product.artifacts, first.product.artifacts);

        let root = dir.root().to_owned();
        let observer = observe_preparation(move |path| {
            if path.file_name().is_some_and(|name| name == "z.css") {
                for extension in ["ts", "mts", "cts"] {
                    write(
                        &root,
                        &format!("parts/nested/added.d.{extension}"),
                        "new invalid declarations!",
                    );
                    write(
                        &root,
                        &format!("parts/value.d.{extension}"),
                        "changed invalid declarations!",
                    );
                }
            }
        });
        let rechecked = session.build(&config).unwrap();
        drop(observer);
        assert!(Rc::ptr_eq(&first.product, &rechecked.product));
        assert_eq!(rechecked.counters.source_parses, 0);
        for extension in ["ts", "mts", "cts"] {
            let added = dir.root().join(format!("parts/nested/added.d.{extension}"));
            assert!(
                added.is_file(),
                "preparation observer must add declarations"
            );
            fs::remove_file(added).unwrap();
            fs::remove_file(dir.root().join(format!("parts/value.d.{extension}"))).unwrap();
        }
        let removed = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&first.product, &removed.product));

        let added = dir.root().join("parts/nested/added.mjs");
        let observer = observe_preparation(move |path| {
            if path.file_name().is_some_and(|name| name == "z.css") {
                fs::write(&added, "export default 2;").unwrap();
            }
        });
        let error = format!("{:#}", session.build(&config).err().unwrap());
        drop(observer);
        assert!(
            error.contains("context candidates changed during build"),
            "{error}"
        );
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
        let changed = session.build(&config).unwrap();
        assert!(!Rc::ptr_eq(&first.product, &changed.product));
        assert!(changed
            .product
            .graph
            .file_id(&dir.root().join("parts/nested/added.mjs"))
            .is_some());
    }
}

#[test]
fn explicit_runtime_declaration_requests_still_fail_with_original_input_errors() {
    for extension in ["ts", "mts", "cts"] {
        let dir = TmpDir::new();
        let file = format!("types.d.{extension}");
        write(
            dir.root(),
            &file,
            "declare const value: number; export {value};",
        );
        for source in [
            format!("import './{file}';"),
            format!("export const load = () => import('./{file}');"),
            "import './loader.cjs';".to_owned(),
        ] {
            write(dir.root(), "main.mjs", &source);
            write(dir.root(), "loader.cjs", &format!("require('./{file}');"));
            let mut session = BuildSession::new(dir.root()).unwrap();
            for _ in 0..2 {
                let error = format!("{:#}", session.build(&options()).err().unwrap());
                assert!(error.contains(&file), "{error}");
                assert!(
                    error.contains("declaration files are not executable inputs"),
                    "{error}"
                );
            }
        }
        let mut config = options();
        config.entries.get_mut("main").unwrap().path = file.into();
        let error = format!(
            "{:#}",
            BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .err()
                .unwrap()
        );
        assert!(
            error.contains("declaration files are not executable inputs"),
            "{error}"
        );
    }
}
