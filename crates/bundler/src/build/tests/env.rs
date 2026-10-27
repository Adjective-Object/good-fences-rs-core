use super::*;

#[test]
fn import_meta_env_default_configured_shared_hygienic_and_cached() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "reader.mjs",
        "export const env = import.meta.env;
        export const read = () => import.meta['env'].changed;",
    );
    write(
        dir.root(),
        "main.mjs",
        "import {env, read} from './reader.mjs';
        const import_meta_env = 'local';
        function probe(import_meta_env) {
            return [import_meta_env, import.meta.env === env,
                import.meta.env ? import.meta.env.MODE : void 0];
        }
        import.meta.env.changed = 7;
        console.log(JSON.stringify([import_meta_env, probe('nested'), read(),
            Object.keys(env).sort(), env.nested, Object.hasOwn(env, '__proto__')]));
        export {env};",
    );
    for target in [TargetEnv::Browser, TargetEnv::Node] {
        for min_chunk_bytes in [0, 10000] {
            let mut config = options();
            config.resolver.target_env = target;
            config.output.min_chunk_bytes = min_chunk_bytes;
            config.output.source_maps = true;
            let mut session = BuildSession::new(dir.root()).unwrap();
            let cold = session.build(&config).unwrap();
            assert_eq!(
                observe(&cold.product.artifacts),
                b"[\"local\",[\"nested\",true,null],7,[\"changed\"],null,false]\n"
            );
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&cold.product, &noop.product));
            assert_eq!(noop.counters.source_parses, 0);
            assert_eq!(noop.counters.emissions, 0);
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(disk.counters.disk_hits, 2);
            assert_eq!(disk.counters.source_reparses, 0);
            assert_eq!(disk.product.artifacts, cold.product.artifacts);
            config.env = BTreeMap::from([
                ("MODE".into(), serde_json::json!("production")),
                (
                    "nested".into(),
                    serde_json::json!({"__proto__": {"value": 1}, "items": [true, null, 2]}),
                ),
                ("__proto__".into(), serde_json::json!("own")),
            ]);
            let configured = session.build(&config).unwrap();
            assert!(!Rc::ptr_eq(&cold.product, &configured.product));
            assert_eq!(configured.counters.memory_hits, 2);
            assert_eq!(configured.counters.source_parses, 1);
            assert_eq!(
                configured.product.fingerprints.parse,
                cold.product.fingerprints.parse
            );
            assert_eq!(
                configured.product.fingerprints.resolution,
                cold.product.fingerprints.resolution
            );
            assert_ne!(
                configured.product.fingerprints.output,
                cold.product.fingerprints.output
            );
            assert_eq!(configured.counters.source_reparses, 0);
            assert!(configured
                .invalidated
                .contains(&dir.root().join("main.mjs")));
            assert_eq!(observe(&configured.product.artifacts),
                b"[\"local\",[\"nested\",true,\"production\"],7,[\"MODE\",\"__proto__\",\"changed\",\"nested\"],{\"__proto__\":{\"value\":1},\"items\":[true,null,2]},true]\n");
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&configured.product, &noop.product));
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(disk.product.artifacts, configured.product.artifacts);
            config.cache = CacheMode::Disabled;
            let disabled = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(disabled.product.artifacts, configured.product.artifacts);
            assert_eq!(disabled.counters.source_reparses, 0);
        }
    }
}

#[test]
fn import_meta_env_does_not_admit_other_metadata_or_commonjs() {
    for (name, source) in [
        ("main.mjs", "export default import.meta;"),
        ("main.mjs", "export default import.meta.other;"),
        ("main.mjs", "export default import.meta[unknown];"),
        ("main.mjs", "export default import.meta.url;"),
        ("main.cjs", "module.exports = import.meta.env;"),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), name, source);
        let mut config = options();
        config.entries.get_mut("main").unwrap().path = name.into();
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains(if name.ends_with("cjs") {
                "mixed CommonJS/ESM"
            } else {
                "import.meta"
            }),
            "{error:#}"
        );
    }
}

#[test]
fn import_meta_env_shares_one_binding_across_entries_and_dynamic_chunks() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "other.mjs",
        "export const env = import.meta.env;
            export function change() { env.count = (env.count || 0) + 1; }",
    );
    write(
        dir.root(),
        "main.mjs",
        "const other = await import('./other.mjs');
            other.change();
            console.log(import.meta.env === other.env, import.meta.env.count);
            export const env = import.meta.env;",
    );
    let mut config = options();
    config.entries.insert(
        "other".into(),
        Entry {
            path: "other.mjs".into(),
            output_name: None,
            namespace: true,
        },
    );
    for min_chunk_bytes in [0, 10000] {
        config.output.min_chunk_bytes = min_chunk_bytes;
        let built = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(observe(&built.product.artifacts), b"true 1\n");
        let env_file = built
            .product
            .graph
            .file_id(&super::super::env::path(dir.root()))
            .unwrap();
        assert_eq!(
            built.product.graph.file_segments(env_file).unwrap().len(),
            1
        );
        assert!(built
            .product
            .artifacts
            .files
            .values()
            .all(|text| !text.contains("import.meta.env")));
    }
}

#[test]
fn import_meta_env_preserves_worker_url_provenance() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export const env = import.meta.env;
            export const worker = () => new Worker(new URL('./worker.mjs', import.meta.url), {type:'module'});");
    write(
        dir.root(),
        "worker.mjs",
        "console.log(import.meta.env.MODE);",
    );
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert!(built
        .product
        .artifacts
        .files
        .values()
        .any(|text| text.contains("import.meta.url")));
    assert!(built
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("import.meta.env")));
    assert!(built.product.artifacts.entries.len() > 1);
}

#[test]
fn import_meta_env_reconstructs_native_external_seams() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import assert from 'node:assert/strict';
        const import_meta_env = 'local';
        assert.equal(import.meta.env.MODE, 'test');
        console.log(import_meta_env, import.meta.env.MODE);",
    );
    let mut config = options();
    config.externals.insert("node:assert/strict".into());
    config.env.insert("MODE".into(), serde_json::json!("test"));
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(observe(&built.product.artifacts), b"local test\n");
}

#[test]
fn import_meta_env_native_external_seams_avoid_nested_capture_across_cache_modes() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import assert from 'node:assert/strict';
        function read(import_meta_env) {
            assert.equal(import_meta_env.MODE, 'parameter');
            return import.meta.env.MODE;
        }
        function readBlock() {
            const import_meta_env = {MODE: 'block'};
            assert.equal(import_meta_env.MODE, 'block');
            return import.meta['env'].MODE;
        }
        function readCatch() {
            try { throw {MODE: 'catch'}; } catch (import_meta_env) {
                assert.equal(import_meta_env.MODE, 'catch');
                return import.meta.env.MODE;
            }
        }
        assert.equal(import.meta.env.MODE, 'test');
        console.log(read({MODE: 'parameter'}), readBlock(), readCatch(), import.meta.env.MODE);",
    );
    let mut config = options();
    config.externals.insert("node:assert/strict".into());
    config.env.insert("MODE".into(), serde_json::json!("test"));
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(observe(&built.product.artifacts), b"test test test test\n");
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&built.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    let mut memory_config = config.clone();
    // Re-emission uses cached source facts, rather than only the no-op path.
    memory_config.output.source_maps = true;
    let memory = session.build(&memory_config).unwrap();
    assert_eq!(memory.counters.memory_hits, 1);
    assert_eq!(observe(&memory.product.artifacts), b"test test test test\n");
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.disk_hits, 1);
    assert_eq!(disk.product.artifacts, built.product.artifacts);
    assert_eq!(observe(&disk.product.artifacts), b"test test test test\n");
    config.cache = CacheMode::Disabled;
    let disabled = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disabled.product.artifacts, built.product.artifacts);
    assert_eq!(
        observe(&disabled.product.artifacts),
        b"test test test test\n"
    );
}
