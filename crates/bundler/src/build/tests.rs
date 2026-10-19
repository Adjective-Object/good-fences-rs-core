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
    let checked = link::esm(graph, &incremental.product.plan, &options.effects, &sources).unwrap();
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
        Path::new("native.mjs")
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
