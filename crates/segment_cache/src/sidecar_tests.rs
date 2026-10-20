use super::*;
use test_tmpdir::TmpDir;

const SOURCE: &str = "import {x as y} from './dep'; export const value = y;";

fn setup() -> (TmpDir, PathBuf) {
    let dir = TmpDir::new();
    let path = dir.root_join("entry.js");
    fs::write(&path, SOURCE).unwrap();
    (dir, path)
}

fn run(path: &Path) -> eyre::Result<CacheOutcome> {
    load_or_parse(
        &logger::StdioLogger::new(),
        path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite,
    )
}

fn no_staging(dir: &TmpDir) {
    assert!(fs::read_dir(dir.root()).unwrap().all(|p| !p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".stage-")));
}

#[test]
fn cold_warm_and_cache_modes() {
    let (_dir, path) = setup();
    let cold = run(&path).unwrap();
    assert_eq!(cold.status, CacheStatus::Miss(MissReason::Missing));
    assert_eq!(cold.write, CacheWrite::Stored);
    let warm = run(&path).unwrap();
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(warm.write, CacheWrite::NotRequested);
    assert_eq!(cold.record, warm.record);
    assert_eq!(cold.source, warm.source);
    let disabled = load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::Disabled,
    )
    .unwrap();
    assert_eq!(disabled.status, CacheStatus::Miss(MissReason::Disabled));
    assert_eq!(disabled.write, CacheWrite::NotRequested);
    fs::remove_file(sidecar_path(&path)).unwrap();
    let readonly = load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(readonly.write, CacheWrite::NotRequested);
    assert!(!sidecar_path(&path).exists());
    assert_eq!(cold.record, readonly.record);
}

#[test]
fn inferred_contracts_roundtrip_and_link_without_reparsing() {
    let dir = TmpDir::new();
    let path = dir.root_join("library.mjs");
    fs::write(
        &path,
        "export function identity(value) { return value; } \
        export function forward(value) { return identity(value); }",
    )
    .unwrap();
    let cold = run(&path).unwrap();
    let bytes = fs::read(sidecar_path(&path)).unwrap();
    let parses = ast_segmenter::module_parse_count();
    let warm = run(&path).unwrap();
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    assert_eq!(cold.record, warm.record);
    let library = warm.record.module.facts.ecco.as_ref().unwrap();
    let contract = library
        .declarations()
        .definitions
        .iter()
        .find(|definition| definition.binding == ecco::Binding::Local("forward".into()))
        .unwrap();
    assert_eq!(contract.returns, ecco::Returns::Argument("value".into()));
    let forward = library
        .symbols()
        .iter()
        .find(|symbol| symbol.name == "forward")
        .unwrap()
        .id;
    let entry = std::sync::Arc::new(
        ecco::parse_source(
            "import {forward} from './library.mjs'; forward({});",
            Path::new("entry.mjs"),
            ecco::LocalFacts::default(),
        )
        .unwrap(),
    );
    for _ in 0..2 {
        let mut analysis = ecco::Analysis::new();
        analysis
            .add_file("library", std::sync::Arc::new(library.clone()))
            .unwrap();
        analysis.add_file("entry", entry.clone()).unwrap();
        let result = analysis
            .solve(&|_: &&str, reference: &ecco::ExternalReference| {
                Ok(match &reference.binding {
                    ecco::Binding::Module { module, .. } if module == "./library.mjs" => {
                        ecco::Resolution::Symbol(ecco::Symbol {
                            file: "library",
                            local: forward,
                        })
                    }
                    _ => ecco::Resolution::Unknown,
                })
            })
            .unwrap();
        assert!(result.files["entry"]
            .roots
            .iter()
            .all(|root| !result.evaluation.effect(root.effects)));
    }
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), bytes);
}

#[test]
fn declaration_changes_invalidate_disk_and_opaque_commonjs() {
    let dir = TmpDir::new();
    for name in ["entry.mjs", "entry.cjs"] {
        let path = dir.root_join(name);
        fs::write(&path, "const value = 1;").unwrap();
        let declarations = ecco::LocalFacts::default();
        let changed =
            ecco::parse_declarations("global fn noop() { call_effects = false }").unwrap();
        let run = |facts| {
            load_or_parse_with_declarations(
                &logger::StdioLogger::new(),
                &path,
                ModuleFormat::Esm,
                CacheMode::ReadWrite,
                facts,
            )
            .unwrap()
        };
        let before = ast_segmenter::module_parse_count();
        assert!(matches!(run(&declarations).status, CacheStatus::Miss(_)));
        assert_eq!(run(&declarations).status, CacheStatus::Hit);
        assert_eq!(ast_segmenter::module_parse_count(), before + 1);
        let changed_record = run(&changed);
        assert!(matches!(
            changed_record.status,
            CacheStatus::Miss(MissReason::InvalidRecord(_))
        ));
        assert!(changed_record.record.matches_declarations(&changed));
        assert!(!changed_record.record.matches_declarations(&declarations));
        assert_eq!(run(&changed).status, CacheStatus::Hit);
        assert_eq!(ast_segmenter::module_parse_count(), before + 2);
    }
}

#[test]
fn corruption_truncation_and_oversize_are_repaired_misses() {
    let (_dir, path) = setup();
    let cold = run(&path).unwrap();
    let bytes = fs::read(sidecar_path(&path)).unwrap();
    for broken in [
        b"not CBOR".to_vec(),
        bytes[..bytes.len() / 2].to_vec(),
        vec![0; MAX_RECORD_BYTES + 1],
    ] {
        fs::write(sidecar_path(&path), broken).unwrap();
        let outcome = run(&path).unwrap();
        assert!(matches!(
            outcome.status,
            CacheStatus::Miss(MissReason::InvalidRecord(_))
        ));
        assert_eq!(outcome.record, cold.record);
        assert_eq!(outcome.write, CacheWrite::Stored);
        assert_eq!(run(&path).unwrap().status, CacheStatus::Hit);
    }
}

#[test]
fn interrupted_publication_preserves_valid_destination_and_cleans_staging() {
    let (dir, path) = setup();
    let cold = run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    let error = publish(&path, SOURCE, &cold.record, || {
        eyre::bail!("injected interruption")
    })
    .unwrap_err();
    assert!(error.to_string().contains("interruption"));
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
    assert!(std::panic::catch_unwind(|| {
        let _ = publish(&path, SOURCE, &cold.record, || panic!("cancelled writer"));
    })
    .is_err());
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
    let mut invalid = cold.record;
    invalid.source_len += 1;
    assert!(publish(&path, SOURCE, &invalid, || Ok(())).is_err());
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
}

#[test]
fn source_change_during_staging_cannot_publish_or_return_stale_success() {
    let (dir, path) = setup();
    run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    fs::write(&path, "export const changed = 1;").unwrap();
    let error = load(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite,
        &ecco::DeclarationConfig::default().effective().unwrap(),
        || {
            fs::write(&path, "export const changed = 2;")?;
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("source changed"));
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
}

#[test]
fn parse_failure_never_resurrects_prior_success() {
    let (_dir, path) = setup();
    run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    for broken in [
        "const = ;",
        "const a = 1; const a = 2;",
        "export const x = import.meta.url;",
    ] {
        fs::write(&path, broken).unwrap();
        assert!(run(&path).is_err());
        assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    }
    fs::write(&path, SOURCE).unwrap();
    assert_eq!(run(&path).unwrap().status, CacheStatus::Hit);
}

#[test]
fn replacement_failure_is_nonfatal_and_does_not_remove_destination() {
    let (dir, path) = setup();
    let destination = sidecar_path(&path);
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), b"do not remove").unwrap();
    let outcome = run(&path).unwrap();
    assert!(matches!(outcome.write, CacheWrite::Failed(_)));
    assert_eq!(
        fs::read(destination.join("keep")).unwrap(),
        b"do not remove"
    );
    assert_eq!(
        outcome.record.module.facts.dependencies[0].specifier,
        "./dep"
    );
    no_staging(&dir);
}

#[test]
fn concurrent_publishers_always_leave_one_complete_record() {
    let (dir, path) = setup();
    let cold = run(&path).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let path = &path;
            let record = &cold.record;
            scope.spawn(move || {
                for _ in 0..8 {
                    publish(path, SOURCE, record, || Ok(())).unwrap();
                    assert_eq!(run(path).unwrap().record, *record);
                }
            });
        }
    });
    assert_eq!(run(&path).unwrap().status, CacheStatus::Hit);
    no_staging(&dir);
}

#[test]
fn deletion_during_staging_preserves_previous_record() {
    let (dir, path) = setup();
    let cold = run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    assert!(publish(&path, SOURCE, &cold.record, || {
        fs::remove_file(&path)?;
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    assert!(run(&path).is_err());
    no_staging(&dir);
}

#[cfg(unix)]
#[test]
fn readonly_source_is_not_modified_and_alias_sidecars_are_lexical() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let (dir, path) = setup();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert_eq!(run(&path).unwrap().write, CacheWrite::Stored);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o444
    );
    let alias = dir.root_join("alias.js");
    symlink(&path, &alias).unwrap();
    assert_eq!(run(&alias).unwrap().write, CacheWrite::Stored);
    assert!(sidecar_path(&alias).is_file());
    let target = dir.root_join("untouched");
    fs::write(&target, b"must not overwrite").unwrap();
    fs::remove_file(sidecar_path(&alias)).unwrap();
    symlink(&target, sidecar_path(&alias)).unwrap();
    assert_eq!(run(&alias).unwrap().write, CacheWrite::Stored);
    assert!(!fs::symlink_metadata(sidecar_path(&alias))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(target).unwrap(), b"must not overwrite");
}

#[cfg(unix)]
#[test]
fn unwritable_parent_reports_uncached_success_and_preserves_old_cache() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, path) = setup();
    run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    fs::write(&path, "export const edited = 1;").unwrap();
    fs::set_permissions(dir.root(), fs::Permissions::from_mode(0o555)).unwrap();
    let probe = dir.root_join("permission-probe");
    let privileged = fs::write(&probe, b"probe").is_ok();
    let result = run(&path);
    fs::set_permissions(dir.root(), fs::Permissions::from_mode(0o755)).unwrap();
    let outcome = result.unwrap();
    if privileged {
        // Root/ACL overrides may legitimately allow the write.
        assert_eq!(outcome.write, CacheWrite::Stored);
    } else {
        assert!(matches!(outcome.write, CacheWrite::Failed(_)));
        assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    }
    assert_eq!(outcome.source, "export const edited = 1;");
    no_staging(&dir);
}

#[test]
fn watcher_exclusions_and_orphan_staging() {
    let (dir, path) = setup();
    let stage = dir.root_join("entry.js.seg.cbor.stage-abandoned");
    fs::write(&stage, b"incomplete").unwrap();
    assert!(is_cache_artifact(&stage));
    assert!(is_cache_artifact(&sidecar_path(&path)));
    assert!(!is_cache_artifact(&path));
    assert_eq!(run(&path).unwrap().write, CacheWrite::Stored);
    assert_eq!(fs::read(&stage).unwrap(), b"incomplete");
}
