use super::*;

const BERRY: &str = r#"
__metadata:
  version: 10
  cacheKey: 10c0
"root@workspace:.":
  version: 0.0.0-use.local
  resolution: "root@workspace:."
  dependencies:
    pkg: "npm:1.0.0"
"pkg@npm:1.0.0":
  version: 1.0.0
  resolution: "pkg@npm:1.0.0"
  checksum: pkg-one
"unused@npm:1.0.0":
  version: 1.0.0
  resolution: "unused@npm:1.0.0"
  checksum: unused-one
"workspace@workspace:packages/workspace":
  version: 0.0.0-use.local
  resolution: "workspace@workspace:packages/workspace"
  dependencies:
    pkg: "npm:1.0.0"
"different@npm:1":
  version: "1"
  resolution: "different@npm:1"
  peerDependencies:
    unknown: "*"
"incomplete@workspace:packages/incomplete":
  version: 0.0.0-use.local
  resolution: "incomplete@workspace:packages/incomplete"
  dependencies:
    different: "npm:1"
"#;

fn fixture(root: &Path) {
    write(
        root,
        "package.json",
        r#"{"name":"root","private":true,"dependencies":{"pkg":"1.0.0"}}"#,
    );
    write(root, "yarn.lock", BERRY);
    write(root, "main.mjs", "export {value} from 'pkg';");
    // Ordinary Berry node_modules manifests do not retain locked locators.
    write(
        root,
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0","exports":"./index.mjs"}"#,
    );
    write(
        root,
        "node_modules/pkg/index.mjs",
        "export {value} from './value.mjs';",
    );
    write(
        root,
        "node_modules/pkg/value.mjs",
        "export const value = 1;",
    );
}

fn build(root: &Path, config: &BuildOptions) -> BuildResult {
    BuildSession::new(root).unwrap().build(config).unwrap()
}

fn sidecar(package: &Path, config: &BuildOptions) -> PathBuf {
    let policy = resolver::BuildResolver::policy(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    package.join(format!(".bundler-resolution-v1-{policy}.fb"))
}

#[test]
fn resolution_cache_new_sessions_and_incomplete_installed_graphs_hit() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let cold = build(dir.root(), &options());
    assert_eq!(cold.counters.resolutions, 2);
    assert_eq!(cold.counters.resolution_cache_misses, 2);
    assert!(sidecar(dir.root(), &options()).is_file());
    assert!(sidecar(&dir.root().join("node_modules/pkg"), &options()).is_file());
    let warm = build(dir.root(), &options());
    assert_eq!(warm.counters.resolutions, 0);
    assert_eq!(warm.counters.resolution_cache_hits, 2);
    assert_eq!(warm.product.artifacts, cold.product.artifacts);

    write(
        dir.root(),
        "packages/incomplete/package.json",
        r#"{"name":"incomplete"}"#,
    );
    write(
        dir.root(),
        "packages/incomplete/main.mjs",
        "export {value} from './value.mjs';",
    );
    write(
        dir.root(),
        "packages/incomplete/value.mjs",
        "export const value = 3;",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "packages/incomplete/main.mjs".into();
    assert_eq!(build(dir.root(), &config).counters.resolutions, 1);
    assert_eq!(build(dir.root(), &config).counters.resolutions, 0);
}

#[test]
#[ignore = "subprocess helper, invoked with a managed project directory"]
fn resolution_cache_process_worker() {
    let root = PathBuf::from(std::env::var_os("BUNDLER_RESOLUTION_TEST_ROOT").unwrap());
    let result = build(&root, &options());
    fs::write(
        root.join(".bundler-process-result"),
        format!(
            "{}:{}:{:?}",
            result.counters.resolutions,
            result.counters.resolution_cache_hits,
            hash(format!("{:?}", result.product.artifacts).as_bytes()),
        ),
    )
    .unwrap();
}

#[test]
fn resolution_cache_separate_processes_reuse_original_request_proofs() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let run = || {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "build::tests::resolution_cache::resolution_cache_process_worker",
                "--ignored",
            ])
            .env("BUNDLER_RESOLUTION_TEST_ROOT", dir.root())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        fs::read_to_string(dir.root().join(".bundler-process-result")).unwrap()
    };
    let cold = run();
    let warm = run();
    assert!(cold.starts_with("2:0:"), "{cold}");
    assert!(warm.starts_with("0:2:"), "{warm}");
    assert_eq!(cold.splitn(3, ':').nth(2), warm.splitn(3, ':').nth(2));
}

#[test]
fn resolution_cache_unrelated_json_and_lock_node_preserve_noop_generation() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&options()).unwrap();
    for value in ["{}", r#"{"new":42}"#] {
        write(dir.root(), "unrelated.json", value);
        write(dir.root(), "tsconfig.json", "not a runtime config");
        write(
            dir.root(),
            "yarn.lock",
            &BERRY.replace("unused-one", &format!("unrelated-{}", value.len())),
        );
        let next = session.build(&options()).unwrap();
        assert!(Rc::ptr_eq(&first.product, &next.product));
        assert!(next.invalidated.is_empty());
        assert_eq!(next.counters.resolutions, 0);
        assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
    }
}

#[test]
fn resolution_cache_dependency_manifest_and_policy_miss_without_reextracting() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = options();
    let first = build(dir.root(), &config);
    write(
        dir.root(),
        "yarn.lock",
        &BERRY.replace("pkg-one", "pkg-two"),
    );
    let dependency = build(dir.root(), &config);
    // The installed package has no explicit locator; its actual proof remains
    // eligible. The mapped workspace's named dependency closure changes.
    assert_eq!(dependency.counters.resolutions, 1);
    assert_eq!(dependency.counters.source_parses, 0);
    assert_eq!(dependency.product.artifacts, first.product.artifacts);
    write(
        dir.root(),
        "package.json",
        r#"{"name":"root","private":true,"description":"mutable","dependencies":{"pkg":"1.0.0"}}"#,
    );
    assert_eq!(build(dir.root(), &config).counters.resolutions, 1);
    let mut changed = config.clone();
    changed.resolver.export_conditions.push("custom".into());
    let policy = build(dir.root(), &changed);
    assert_eq!(policy.counters.resolutions, 2);
    assert_eq!(policy.counters.source_parses, 0);
    assert_ne!(
        policy.product.fingerprints.resolution,
        dependency.product.fingerprints.resolution
    );

    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0","exports":"./other.mjs"}"#,
    );
    write(
        dir.root(),
        "node_modules/pkg/other.mjs",
        "export const value = 2;",
    );
    let redirected = build(dir.root(), &config);
    assert_eq!(redirected.counters.resolutions, 1);
    assert_ne!(redirected.product.artifacts, first.product.artifacts);
}

#[test]
fn resolution_cache_explicit_installed_locator_tracks_its_named_locked_closure() {
    let dir = TmpDir::new();
    fixture(dir.root());
    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","version":"1.0.0","_locator":"pkg@npm:1.0.0","exports":"./index.mjs"}"#,
    );
    build(dir.root(), &options());
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
    write(
        dir.root(),
        "yarn.lock",
        &BERRY.replace("unused-one", "unrelated"),
    );
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
    write(
        dir.root(),
        "yarn.lock",
        &BERRY.replace("pkg-one", "changed"),
    );
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 2);
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
}

#[test]
fn resolution_cache_request_base_mode_and_shadow_proofs_are_independent() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs",
        "import {value as a} from './a/use.mjs'; import {value as b} from './b/use.mjs'; export {a,b};");
    write(dir.root(), "a/use.mjs", "export {value} from './value';");
    write(dir.root(), "b/use.mjs", "export {value} from './value';");
    write(dir.root(), "a/value.ts", "export const value = 1;");
    write(dir.root(), "b/value.ts", "export const value = 2;");
    let first = build(dir.root(), &options());
    assert_eq!(first.counters.resolutions, 4);
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
    write(dir.root(), "a/value.js", "export const value = 3;");
    let shadow = build(dir.root(), &options());
    assert_eq!(shadow.counters.resolutions, 1);
    assert_ne!(shadow.product.artifacts, first.product.artifacts);
    assert_eq!(shadow.counters.resolution_cache_hits, 3);

    write(
        dir.root(),
        "main.mjs",
        "export {default} from './modes.cjs';",
    );
    write(
        dir.root(),
        "modes.cjs",
        "module.exports = {esm: () => import('choice'), cjs: require('choice')};",
    );
    write(
        dir.root(),
        "node_modules/choice/package.json",
        r#"{"exports":{".":{"import":"./import.mjs","require":"./require.cjs"}}}"#,
    );
    write(
        dir.root(),
        "node_modules/choice/import.mjs",
        "export default 1;",
    );
    write(
        dir.root(),
        "node_modules/choice/require.cjs",
        "module.exports = 2;",
    );
    let modes = build(dir.root(), &options());
    let warm = build(dir.root(), &options());
    assert_eq!(warm.counters.resolutions, 0);
    assert_eq!(warm.product.artifacts, modes.product.artifacts);
    assert!(warm
        .product
        .graph
        .file_id(&dir.root().join("node_modules/choice/import.mjs"))
        .is_some());
    assert!(warm
        .product
        .graph
        .file_id(&dir.root().join("node_modules/choice/require.cjs"))
        .is_some());
}

#[test]
fn resolution_cache_aggregate_preserves_other_request_records() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export {value} from './a.mjs';");
    write(dir.root(), "other.mjs", "export {value} from './b.mjs';");
    write(dir.root(), "a.mjs", "export const value = 1;");
    write(dir.root(), "b.mjs", "export const value = 2;");
    let config = options();
    build(dir.root(), &config);
    let mut other = config.clone();
    other.entries.get_mut("main").unwrap().path = "other.mjs".into();
    assert_eq!(build(dir.root(), &other).counters.resolutions, 1);
    assert_eq!(build(dir.root(), &config).counters.resolutions, 0);
    assert_eq!(build(dir.root(), &other).counters.resolutions, 0);
}

#[test]
fn resolution_cache_corrupt_readonly_disabled_and_unwritable_preserve_results() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = options();
    let first = build(dir.root(), &config);
    let path = sidecar(dir.root(), &config);
    fs::write(&path, b"broken").unwrap();
    let mut readonly = config.clone();
    readonly.cache = CacheMode::ReadOnly;
    let recovered = build(dir.root(), &readonly);
    assert_eq!(recovered.counters.resolutions, 1);
    assert_eq!(fs::read(&path).unwrap(), b"broken");
    assert!(recovered
        .warnings
        .iter()
        .any(|w| w.contains("resolution cache miss")));
    assert_eq!(recovered.product.artifacts, first.product.artifacts);
    let mut disabled = config.clone();
    disabled.cache = CacheMode::Disabled;
    let uncached = build(dir.root(), &disabled);
    assert_eq!(uncached.counters.resolutions, 2);
    assert_eq!(uncached.counters.resolution_cache_hits, 0);
    assert!(!uncached
        .warnings
        .iter()
        .any(|w| w.contains("resolution cache")));
    assert_eq!(fs::read(&path).unwrap(), b"broken");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let unwritable = build(dir.root(), &config);
    assert_eq!(unwritable.product.artifacts, first.product.artifacts);
    assert!(unwritable
        .warnings
        .iter()
        .any(|w| w.contains("resolution cache write failed")));
    fs::remove_dir(&path).unwrap();
    let repaired = build(dir.root(), &config);
    assert_eq!(repaired.counters.resolutions, 1);
    let hit = build(dir.root(), &readonly);
    assert_eq!(hit.counters.resolutions, 0);
    assert_eq!(hit.product.artifacts, first.product.artifacts);
}

#[test]
fn resolution_cache_rechecks_disk_proofs_and_lockfile_before_noop_or_publication() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&options()).unwrap();
    let path = sidecar(dir.root(), &options());
    let before = fs::read(&path).unwrap();
    let root = dir.root().to_owned();
    let observer = observe_preparation(move |path| {
        if path.file_name().is_some_and(|name| name == "value.mjs") {
            // The earlier package request is now a disk/memory hit. Even an
            // equal target must not accept changed consulted manifest bytes.
            fs::write(
                root.join("node_modules/pkg/package.json"),
                r#"{"name":"pkg","version":"1.0.0","exports":"./index.mjs","changed":true}"#,
            )
            .unwrap();
        }
    });
    assert!(session.build(&options()).is_err());
    drop(observer);
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    assert_eq!(fs::read(&path).unwrap(), before);
    fixture(dir.root());
    let root = dir.root().to_owned();
    let observer = observe_preparation(move |_| {
        fs::write(
            root.join("yarn.lock"),
            BERRY.replace("pkg-one", "during-build"),
        )
        .unwrap();
    });
    assert!(session.build(&options()).is_err());
    drop(observer);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn resolution_cache_missing_or_unparseable_lockfile_uses_actual_proofs() {
    let dir = TmpDir::new();
    fixture(dir.root());
    fs::remove_file(dir.root().join("yarn.lock")).unwrap();
    let first = build(dir.root(), &options());
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 0);
    write(dir.root(), "yarn.lock", "__metadata: [invalid");
    let fallback = build(dir.root(), &options());
    assert_eq!(fallback.product.artifacts, first.product.artifacts);
    assert!(fallback
        .warnings
        .iter()
        .any(|w| w.contains("lockfile unavailable")));
    write(
        dir.root(),
        "node_modules/pkg/package.json",
        r#"{"exports":"./value.mjs"}"#,
    );
    assert_eq!(build(dir.root(), &options()).counters.resolutions, 1);
}

#[test]
#[cfg(windows)]
fn resolution_cache_unreadable_lockfile_is_not_resolution_authority() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = TmpDir::new();
    fixture(dir.root());
    let first = build(dir.root(), &options());
    let _exclusive = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.root().join("yarn.lock"))
        .unwrap();
    let fallback = build(dir.root(), &options());
    assert_eq!(fallback.product.artifacts, first.product.artifacts);
    assert!(fallback
        .warnings
        .iter()
        .any(|w| w.contains("lockfile unavailable")));
}
