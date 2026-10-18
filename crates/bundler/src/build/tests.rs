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

fn parity(session: &mut BuildSession, options: &BuildOptions) -> BuildResult {
    let incremental = session.build(options).unwrap();
    let full = BuildSession::new(session.root())
        .unwrap()
        .build(options)
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
    incremental
}

fn observe(artifacts: &Artifacts) -> Vec<u8> {
    let dir = TmpDir::new();
    for (path, text) in &artifacts.files {
        write(dir.root(), path.to_str().unwrap(), text);
    }
    let output = std::process::Command::new("node")
        .args(["-e", "const {spawnSync}=require('node:child_process'); const r=spawnSync(process.execPath,[process.argv[1]],{timeout:5000,killSignal:'SIGKILL'}); if(r.error||r.status!==0||r.stderr.length) {console.error(r.error||r.stderr.toString());process.exit(1)} process.stdout.write(r.stdout)"])
        .arg(dir.root().join(&artifacts.entries["main"]))
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
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
    assert_eq!(observe(&cold.product.artifacts), b"1\n");
    let same = parity(&mut session, &options);
    assert_eq!(same.counters.source_parses, 0);
    assert_eq!(same.counters.resolutions, 0);
    assert_eq!(same.counters.emissions, 0);
    assert!(Rc::ptr_eq(&same.product, &cold.product));
    let warm = BuildSession::new(root).unwrap().build(&options).unwrap();
    assert_eq!(warm.counters.source_parses, 0);
    assert_eq!(warm.counters.disk_hits, 2);
    options.effects = EffectPolicy::parse(["*.appendChild(...)"]).unwrap();
    let policy = parity(&mut session, &options);
    assert_eq!(policy.counters.source_parses, 0);
    assert_eq!(policy.counters.resolutions, 0);
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
    assert_eq!(roots.counters.source_parses, 0);
    assert_eq!(roots.counters.resolutions, 0);
    assert_ne!(
        roots.product.fingerprints.roots,
        policy.product.fingerprints.roots
    );
    write(root, "dep.mjs", "export const value = 2;");
    let local = parity(&mut session, &options);
    assert_eq!(local.counters.source_parses, 1);
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
