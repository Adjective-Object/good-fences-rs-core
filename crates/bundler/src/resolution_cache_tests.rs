use super::*;
use ftree_cache::inputs::InputObserver;
use std::fs;

const KEY: Key = Key {
    policy: [1; 32],
    instance: [2; 32],
    dependencies: [3; 32],
};

fn fixture() -> (tempfile::TempDir, Roots, InputProof, PathBuf) {
    let test_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("test-temp");
    fs::create_dir_all(&test_root).unwrap();
    let dir = tempfile::tempdir_in(test_root).unwrap();
    let root = dir.path().canonicalize().unwrap().join("project");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/main.js"), "import './target'").unwrap();
    fs::write(root.join("pkg/target.js"), "export {}").unwrap();
    fs::write(root.join("pkg/package.json"), "{}").unwrap();
    let roots = Roots::new(&root, &root.join("pkg")).unwrap();
    let observer = InputObserver::default();
    observer.record(PathInput::capture(&root.join("pkg/target")).unwrap());
    observer.record(PathInput::capture(&root.join("pkg/target.js")).unwrap());
    let mut manifest = PathInput::capture(&root.join("pkg/package.json")).unwrap();
    manifest.seal_content(b"{}");
    observer.record(manifest);
    (dir, roots, observer.proof(), root.join("pkg/target.js"))
}

fn record(roots: Roots, proof: &InputProof, target: &Path, key: Key) -> CheckedResolution {
    CheckedResolution::encode(
        roots,
        key,
        &[
            Request {
                mode: wire::Mode::Require,
                importer: "main.js",
                specifier: "node:fs",
                target: Target::External("node:fs"),
                inputs: proof,
            },
            Request {
                mode: wire::Mode::Import,
                importer: "main.js",
                specifier: "./target",
                target: Target::File(target),
                inputs: proof,
            },
        ],
    )
    .unwrap()
}

#[test]
fn disk_roundtrip_borrows_and_revalidates_positive_negative_and_content_inputs() {
    let (_dir, roots, proof, target) = fixture();
    let record = record(roots.clone(), &proof, &target, KEY);
    let store = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(store.load().status, CacheStatus::Miss(MissReason::Missing));
    assert_eq!(store.save(&record), CacheWrite::Stored);
    let loaded = store.load();
    assert_eq!(loaded.status, CacheStatus::Hit);
    let loaded = loaded.record.unwrap();
    assert_eq!(
        loaded
            .requests()
            .map(|request| request.mode())
            .collect::<Vec<_>>(),
        vec![wire::Mode::Import, wire::Mode::Require]
    );
    let request = loaded
        .find(wire::Mode::Import, "main.js", "./target")
        .unwrap();
    assert_eq!(
        loaded
            .file_target(wire::Mode::Import, "main.js", "./target")
            .unwrap(),
        Some(target.clone())
    );
    let pointer = request.specifier().as_ptr() as usize;
    let allocation = loaded.wire.bytes().as_ptr() as usize;
    assert!((allocation..allocation + loaded.wire.bytes().len()).contains(&pointer));
    assert_eq!(
        loaded
            .find(wire::Mode::Require, "main.js", "node:fs")
            .unwrap()
            .target()
            .external(),
        Some("node:fs")
    );
    assert!(loaded
        .find(wire::Mode::Require, "main.js", "./target")
        .is_none());
    let shared = loaded.clone();
    drop(loaded);
    assert_eq!(
        shared
            .find(wire::Mode::Import, "main.js", "./target")
            .unwrap()
            .specifier()
            .as_ptr() as usize,
        pointer
    );
    // Candidate source contents are deliberately not resolution inputs.
    fs::write(&target, "export const changed = 1").unwrap();
    shared.revalidate().unwrap();
    fs::write(roots.package.join("target"), "nearer priority candidate").unwrap();
    assert!(shared.revalidate().is_err());
    assert!(matches!(
        store.load().status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
    fs::remove_file(roots.package.join("target")).unwrap();
    fs::write(
        roots.package.join("package.json"),
        "{\"main\":\"other.js\"}",
    )
    .unwrap();
    assert!(shared.revalidate().is_err());
}

#[test]
fn cache_modes_policies_and_fingerprints_are_independent() {
    let (_dir, roots, proof, target) = fixture();
    let record_a = record(roots.clone(), &proof, &target, KEY);
    let disabled = Store::new(roots.clone(), KEY, CacheMode::Disabled);
    let readonly = Store::new(roots.clone(), KEY, CacheMode::ReadOnly);
    let writable = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(
        disabled.load().status,
        CacheStatus::Miss(MissReason::Disabled)
    );
    assert_eq!(disabled.save(&record_a), CacheWrite::NotRequested);
    assert_eq!(readonly.save(&record_a), CacheWrite::NotRequested);
    assert!(!writable.path().exists());
    assert_eq!(writable.save(&record_a), CacheWrite::Stored);
    assert_eq!(readonly.load().status, CacheStatus::Hit);
    let key_b = Key {
        policy: [8; 32],
        ..KEY
    };
    let record_b = record(roots.clone(), &proof, &target, key_b);
    let store_b = Store::new(roots.clone(), key_b, CacheMode::ReadWrite);
    assert_ne!(writable.path(), store_b.path());
    assert_eq!(store_b.save(&record_b), CacheWrite::Stored);
    assert_eq!(writable.load().status, CacheStatus::Hit);
    assert_eq!(store_b.load().status, CacheStatus::Hit);
    assert!(matches!(store_b.save(&record_a), CacheWrite::Failed(_)));
    for key in [
        Key {
            instance: [9; 32],
            ..KEY
        },
        Key {
            dependencies: [9; 32],
            ..KEY
        },
    ] {
        assert!(matches!(
            Store::new(roots.clone(), key, CacheMode::ReadOnly)
                .load()
                .status,
            CacheStatus::Miss(MissReason::InvalidRecord(_))
        ));
    }
}

#[test]
fn corrupt_read_and_atomic_publication_failures_have_explicit_diagnostics() {
    let (_dir, roots, proof, target) = fixture();
    let record = record(roots.clone(), &proof, &target, KEY);
    let store = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(store.save(&record), CacheWrite::Stored);
    let original = fs::read(store.path()).unwrap();
    fs::write(store.path(), b"not a flatbuffer").unwrap();
    assert!(matches!(
        store.load().status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
    assert_eq!(store.save(&record), CacheWrite::Stored);
    assert_eq!(fs::read(store.path()).unwrap(), original);
    fs::remove_file(store.path()).unwrap();
    // Deterministic on Windows too: a nonreplaceable publication destination.
    fs::create_dir(store.path()).unwrap();
    assert!(matches!(store.save(&record), CacheWrite::Failed(_)));
    assert!(matches!(
        store.load().status,
        CacheStatus::Miss(MissReason::ReadFailure(_))
            | CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
    assert!(fs::read_dir(&roots.package).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bundler-resolution-stage-")));
}

#[test]
fn replacement_is_atomic_for_readers_and_does_not_change_existing_owners() {
    let (_dir, roots, proof, target) = fixture();
    let record = record(roots.clone(), &proof, &target, KEY);
    let store = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(store.save(&record), CacheWrite::Stored);
    let held = store.load().record.unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..12 {
                assert_eq!(store.load().status, CacheStatus::Hit);
            }
        });
        for _ in 0..12 {
            // Windows can deny replacement while a reader holds the destination.
            // A failed rename must preserve the old complete file, never unlink it.
            match store.save(&record) {
                CacheWrite::Stored => {}
                CacheWrite::Failed(diagnostic) => assert!(!diagnostic.is_empty()),
                CacheWrite::NotRequested => panic!("write unexpectedly disabled"),
            }
        }
    });
    assert_eq!(store.save(&record), CacheWrite::Stored);
    held.revalidate().unwrap();
}

#[test]
fn external_footprints_unproven_inputs_and_duplicate_requests_are_not_persistable() {
    let (_dir, roots, proof, target) = fixture();
    assert!(roots
        .encode(&roots.project.parent().unwrap().join("unrelated/private.js"))
        .is_err());
    let observer = InputObserver::default();
    observer.error("unobserved legacy reader");
    let unproven = observer.proof();
    assert!(CheckedResolution::encode(
        roots.clone(),
        KEY,
        &[Request {
            mode: wire::Mode::Import,
            importer: "main.js",
            specifier: "./target",
            target: Target::File(&target),
            inputs: &unproven,
        }]
    )
    .is_err());
    let request = || Request {
        mode: wire::Mode::Import,
        importer: "main.js",
        specifier: "./target",
        target: Target::File(&target),
        inputs: &proof,
    };
    assert!(CheckedResolution::encode(roots, KEY, &[request(), request()]).is_err());
}

#[test]
fn ancestor_node_lookup_paths_are_portable_and_bounded() {
    let (_dir, roots, _, _) = fixture();
    let ancestor = roots.project.parent().unwrap();
    let path = ancestor.join("node_modules/optional/index.js");
    let encoded = roots.encode(&path).unwrap();
    assert_eq!(encoded.parents, 1);
    assert_eq!(encoded.path, "node_modules/optional/index.js");
    assert!(!encoded.path.contains(':'));
    assert_eq!(encoded.anchor, wire::Anchor::Project);
}

#[test]
fn project_relocation_keeps_relative_paths_and_reinstall_identity_changes_miss() {
    let (dir, roots, proof, target) = fixture();
    let record = record(roots.clone(), &proof, &target, KEY);
    let store = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(store.save(&record), CacheWrite::Stored);
    let moved = dir.path().canonicalize().unwrap().join("relocated");
    fs::rename(&roots.project, &moved).unwrap();
    let roots = Roots::new(&moved, &moved.join("pkg")).unwrap();
    let relocated = Store::new(roots.clone(), KEY, CacheMode::ReadOnly);
    assert_eq!(relocated.load().status, CacheStatus::Hit);
    let replacement = roots.package.join("replacement.js");
    fs::write(&replacement, "export {}").unwrap();
    fs::rename(
        roots.package.join("target.js"),
        roots.package.join("old.js"),
    )
    .unwrap();
    fs::rename(replacement, roots.package.join("target.js")).unwrap();
    assert!(matches!(
        relocated.load().status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
}

#[test]
fn local_directory_links_roundtrip_and_retargeting_invalidates() {
    let (_dir, roots, _, _) = fixture();
    fs::create_dir(roots.project.join("one")).unwrap();
    fs::create_dir(roots.project.join("two")).unwrap();
    for name in ["one", "two"] {
        fs::write(roots.project.join(name).join("linked.js"), "export {}").unwrap();
    }
    let link = roots.package.join("linked");
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(roots.project.join("one"))
            .output()
            .unwrap();
        assert!(status.status.success(), "{:?}", status);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink("../one", &link).unwrap();
    let candidate = link.join("linked.js");
    let target = candidate.canonicalize().unwrap();
    let observer = InputObserver::default();
    observer.record(PathInput::capture(&candidate).unwrap());
    let proof = observer.proof();
    let record = record(roots.clone(), &proof, &target, KEY);
    let store = Store::new(roots.clone(), KEY, CacheMode::ReadWrite);
    assert_eq!(store.save(&record), CacheWrite::Stored);
    assert_eq!(store.load().status, CacheStatus::Hit);
    #[cfg(windows)]
    {
        fs::remove_dir(&link).unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(roots.project.join("two"))
            .output()
            .unwrap();
        assert!(status.status.success(), "{:?}", status);
    }
    #[cfg(unix)]
    {
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("../two", &link).unwrap();
    }
    assert!(matches!(
        store.load().status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
}

#[test]
fn actual_stage_two_package_and_external_footprints_are_persistable() {
    use import_resolver::{
        node_resolver::caching::NodeModulesResolverOptions,
        structural::{ResolutionMode, ResolvedDependency, StructuralResolver},
    };
    let (_dir, roots, _, _) = fixture();
    let dependency = roots.project.join("node_modules").join("dep");
    fs::create_dir_all(&dependency).unwrap();
    fs::write(dependency.join("package.json"), "{\"main\":\"index.js\"}").unwrap();
    fs::write(dependency.join("index.js"), "export {}").unwrap();
    let resolver = StructuralResolver::new(
        roots.project.clone(),
        NodeModulesResolverOptions::default_for_env(import_resolver::resolve::TargetEnv::Node),
        ["node:fs".into()].into(),
    );
    let importer = roots.package.join("main.js");
    let dependency = resolver
        .resolve_dependency_with_inputs(&importer, "dep", ResolutionMode::Import)
        .unwrap();
    let ResolvedDependency::File(target) = dependency.dependency else {
        panic!("expected file")
    };
    let target = target.canonicalize().unwrap();
    let external = resolver
        .resolve_dependency_with_inputs(&importer, "node:fs", ResolutionMode::Require)
        .unwrap();
    assert!(external.inputs.inputs().is_empty());
    let record = CheckedResolution::encode(
        roots.clone(),
        KEY,
        &[
            Request {
                mode: wire::Mode::Import,
                importer: "main.js",
                specifier: "dep",
                target: Target::File(&target),
                inputs: &dependency.inputs,
            },
            Request {
                mode: wire::Mode::Require,
                importer: "main.js",
                specifier: "node:fs",
                target: Target::External("node:fs"),
                inputs: &external.inputs,
            },
        ],
    )
    .unwrap();
    let store = Store::new(roots, KEY, CacheMode::ReadWrite);
    assert_eq!(store.save(&record), CacheWrite::Stored);
    assert_eq!(store.load().status, CacheStatus::Hit);
}

#[test]
fn retargeted_package_anchor_invalidates_even_canonical_only_proofs() {
    let (_dir, roots, proof, target) = fixture();
    let alias = roots.project.join("package-link");
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&roots.package)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&roots.package, &alias).unwrap();
    let anchored = Roots::new(&roots.project, &alias).unwrap();
    let record = record(anchored, &proof, &target, KEY);
    record.revalidate().unwrap();
    let other = roots.project.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("main.js"), "").unwrap();
    #[cfg(windows)]
    {
        fs::remove_dir(&alias).unwrap();
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&other)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[cfg(unix)]
    {
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&other, &alias).unwrap();
    }
    assert!(record
        .revalidate()
        .unwrap_err()
        .to_string()
        .contains("anchors changed"));
}
