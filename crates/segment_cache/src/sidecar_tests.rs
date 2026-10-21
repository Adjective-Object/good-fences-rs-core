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
    let bytes = fs::read(sidecar_path(&path)).unwrap();
    let modified = fs::metadata(sidecar_path(&path))
        .unwrap()
        .modified()
        .unwrap();
    let parses = ast_segmenter::module_parse_count();
    let warm = run(&path).unwrap();
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(warm.write, CacheWrite::NotRequested);
    assert_eq!(cold.record, warm.record);
    assert_eq!(cold.source, warm.source);
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), bytes);
    assert_eq!(
        fs::metadata(sidecar_path(&path))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    let parsed = warm.record.to_parsed(SOURCE).unwrap();
    assert!(parsed.owned_facts().is_err());
    assert!(parsed.owned_segments().is_none());
    assert!(matches!(
        parsed.facts(),
        ast_segmenter::views::ModuleFacts::Wire(_)
    ));
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
fn prepared_sources_cache_generated_coordinates_and_recheck_original_bytes() {
    let dir = TmpDir::new();
    let path = dir.root_join("data.json");
    let original = br#"{"value": 1}"#;
    let source = r#"module.exports = ({["value"]: 1});"#;
    let parser_path = path.with_extension("json.cjs");
    fs::write(&path, original).unwrap();
    let declarations = ecco::LocalFacts::default();
    let prepared = || PreparedSource {
        source,
        original,
        parser_path: &parser_path,
        package: ModuleFormat::CommonJs,
    };
    let cold = load_prepared(
        &logger::StdioLogger::new(),
        &path,
        prepared(),
        CacheMode::ReadWrite,
        &declarations,
    )
    .unwrap();
    let parses = ast_segmenter::module_parse_count();
    let warm = load_prepared(
        &logger::StdioLogger::new(),
        &path,
        prepared(),
        CacheMode::ReadOnly,
        &declarations,
    )
    .unwrap();
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(warm.source, source);
    assert_eq!(warm.record, cold.record);
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    assert!(warm.record.to_parsed(source).is_ok());
    assert!(warm
        .record
        .to_parsed(std::str::from_utf8(original).unwrap())
        .is_err());
    fs::remove_file(sidecar_path(&path)).unwrap();
    let error = load_prepared_inner(
        &logger::StdioLogger::new(),
        &path,
        prepared(),
        CacheMode::ReadWrite,
        &ast_segmenter::PreparedDeclarations::new(declarations).unwrap(),
        || {
            fs::write(&path, b"changed input")?;
            Ok(())
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("source changed"));
    assert!(!sidecar_path(&path).exists());
    no_staging(&dir);
}

#[test]
fn prepared_css_hits_recheck_originals_but_key_generated_text() {
    let dir = TmpDir::new();
    let path = dir.root_join("style.module.css");
    let parser_path = path.with_extension("css.mjs");
    let original = b".button { color: red }";
    let edited = b".button { color: blue }";
    let source = "export const button = '_button';";
    let declarations = ecco::LocalFacts::default();
    fs::write(&path, original).unwrap();
    let load = |source, original, mode| {
        load_prepared(
            &logger::StdioLogger::new(),
            &path,
            PreparedSource {
                source,
                original,
                parser_path: &parser_path,
                package: ModuleFormat::Esm,
            },
            mode,
            &declarations,
        )
    };
    load(source, original.as_slice(), CacheMode::ReadWrite).unwrap();
    let parses = ast_segmenter::module_parse_count();
    fs::write(&path, edited).unwrap();
    let hit = load(source, edited.as_slice(), CacheMode::ReadOnly).unwrap();
    assert_eq!(hit.status, CacheStatus::Hit);
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    let error = load(source, original.as_slice(), CacheMode::ReadOnly).unwrap_err();
    assert!(format!("{error:#}").contains("source changed"));
    let changed = load(
        "export const button = '_different';",
        edited.as_slice(),
        CacheMode::ReadWrite,
    )
    .unwrap();
    assert!(matches!(changed.status, CacheStatus::Miss(_)));
    assert_eq!(ast_segmenter::module_parse_count(), parses + 1);
}

#[test]
fn prepared_css_parser_modes_key_disk_reuse_even_when_generated_text_is_unchanged() {
    let dir = TmpDir::new();
    let path = dir.root_join("style.css");
    let original = b".button { color: red; }";
    let source = "const button = '_button';";
    fs::write(&path, original).unwrap();
    let declarations = ecco::LocalFacts::default();
    for extension in ["css.cjs", "css.mjs", "css.cjs"] {
        let parser_path = path.with_extension(extension);
        let load = |mode| {
            load_prepared(
                &logger::StdioLogger::new(),
                &path,
                PreparedSource {
                    source,
                    original,
                    parser_path: &parser_path,
                    // The prepared extension, not the original CSS/package,
                    // determines the effective parser mode.
                    package: ModuleFormat::Esm,
                },
                mode,
                &declarations,
            )
            .unwrap()
        };
        let before = ast_segmenter::module_parse_count();
        let cold = load(CacheMode::ReadWrite);
        assert!(matches!(cold.status, CacheStatus::Miss(_)));
        assert_eq!(cold.record.config().commonjs(), extension == "css.cjs");
        let warm = load(CacheMode::ReadOnly);
        assert_eq!(warm.status, CacheStatus::Hit);
        assert_eq!(warm.record, cold.record);
        assert_eq!(ast_segmenter::module_parse_count(), before + 1);
        let parsed = warm.record.to_parsed(source).unwrap();
        assert_eq!(parsed.provenance().config(), warm.record.config());
        let other_path = path.with_extension(if extension == "css.cjs" { "mjs" } else { "cjs" });
        let other_config = ParserConfig::for_path(&other_path, ModuleFormat::Esm).unwrap();
        assert!(
            CacheRecord::decode(&warm.record.encode(source).unwrap(), source, other_config)
                .is_err()
        );
    }
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
    let library = warm.record.module().facts().ecco().unwrap();
    let contract = library
        .declarations()
        .definitions()
        .iter()
        .find(|definition| {
            matches!(
                definition.binding(),
                ecco::views::BindingView::Local("forward")
            )
        })
        .unwrap();
    assert!(matches!(
        contract.returns(),
        ecco::views::ReturnsView::Argument("value")
    ));
    let forward = library
        .symbols()
        .iter()
        .find(|symbol| symbol.name() == "forward")
        .unwrap()
        .id();
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
        let storage = warm
            .record
            .to_parsed(&fs::read_to_string(&path).unwrap())
            .unwrap()
            .into_facts();
        analysis
            .add_file("library", storage.ecco_source().unwrap().unwrap())
            .unwrap();
        analysis.add_file("entry", entry.clone()).unwrap();
        let result = analysis
            .solve(
                &|_: &&str, reference: ecco::views::ExternalReferenceView<'_>| {
                    Ok(match reference.binding() {
                        ecco::views::BindingView::Module { module, .. }
                            if module == "./library.mjs" =>
                        {
                            ecco::Resolution::Symbol(ecco::Symbol {
                                file: "library",
                                local: forward,
                            })
                        }
                        _ => ecco::Resolution::Unknown,
                    })
                },
            )
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
        b"not a FlatBuffer".to_vec(),
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
    let error = publish(&path, SOURCE, SOURCE.as_bytes(), &cold.record, || {
        eyre::bail!("injected interruption")
    })
    .unwrap_err();
    assert!(error.to_string().contains("interruption"));
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
    assert!(std::panic::catch_unwind(|| {
        let _ = publish(&path, SOURCE, SOURCE.as_bytes(), &cold.record, || {
            panic!("cancelled writer")
        });
    })
    .is_err());
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
    assert!(publish(
        &path,
        "different source",
        SOURCE.as_bytes(),
        &cold.record,
        || Ok(())
    )
    .is_err());
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), previous);
    no_staging(&dir);
}

#[test]
fn valid_extraction_exceeding_wire_limits_remains_an_uncached_success() {
    let (dir, path) = setup();
    run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    // The binding and Ecco symbol both persist this name, exceeding the wire
    // limit without needing a large/deep syntax tree or excessive source size.
    let source = format!("export const {} = 1;", "x".repeat(MAX_RECORD_BYTES / 2));
    fs::write(&path, &source).unwrap();
    let outcome = run(&path).unwrap();
    assert!(matches!(outcome.status, CacheStatus::Miss(_)));
    assert!(matches!(outcome.write, CacheWrite::Failed(_)));
    assert_eq!(outcome.source, source);
    assert!(outcome
        .record
        .to_parsed(&source)
        .unwrap()
        .owned_facts()
        .is_ok());
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
        &ast_segmenter::PreparedDeclarations::from_config(&ecco::DeclarationConfig::default())
            .unwrap(),
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
        outcome
            .record
            .module()
            .facts()
            .dependencies()
            .get(0)
            .unwrap()
            .specifier(),
        "./dep"
    );
    no_staging(&dir);
}

#[test]
fn concurrent_publishers_always_leave_one_complete_record() {
    let (dir, path) = setup();
    let cold = run(&path).unwrap();
    let mut previous = File::open(sidecar_path(&path)).unwrap();
    let previous_bytes = fs::read(sidecar_path(&path)).unwrap();
    let start = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let path = &path;
            let record = &cold.record;
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for _ in 0..8 {
                    publish(path, SOURCE, SOURCE.as_bytes(), record, || Ok(())).unwrap();
                    assert_eq!(run(path).unwrap().record, *record);
                }
            });
        }
    });
    let mut retained = Vec::new();
    previous.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, previous_bytes);
    drop(previous);
    assert_eq!(run(&path).unwrap().status, CacheStatus::Hit);
    no_staging(&dir);
}

#[test]
fn deletion_during_staging_preserves_previous_record() {
    let (dir, path) = setup();
    let cold = run(&path).unwrap();
    let previous = fs::read(sidecar_path(&path)).unwrap();
    assert!(publish(&path, SOURCE, SOURCE.as_bytes(), &cold.record, || {
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
    let legacy = dir.root_join("entry.js.seg.cbor");
    fs::write(&legacy, b"legacy user file").unwrap();
    let stages = [
        dir.root_join("entry.js.seg.cbor.stage-abandoned"),
        dir.root_join("entry.js.seg.fb.stage-abandoned"),
    ];
    for stage in &stages {
        fs::write(stage, b"incomplete").unwrap();
        assert!(is_cache_artifact(stage));
    }
    assert!(is_cache_artifact(&legacy));
    assert!(is_cache_artifact(&sidecar_path(&path)));
    assert!(!is_cache_artifact(&path));
    let cold = run(&path).unwrap();
    assert_eq!(cold.status, CacheStatus::Miss(MissReason::Missing));
    assert_eq!(cold.write, CacheWrite::Stored);
    assert_eq!(fs::read(&legacy).unwrap(), b"legacy user file");
    for stage in &stages {
        assert_eq!(fs::read(stage).unwrap(), b"incomplete");
    }
    assert_eq!(run(&path).unwrap().status, CacheStatus::Hit);
}
