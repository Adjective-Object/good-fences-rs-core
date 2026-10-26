use super::*;

const CSS: &str = r#"
@import "./theme.css" layer(base);
:global(.theme, .dark) :local(.button) {
  composes: base from "./base.module.css";
  animation: local(spin) 1s;
  background: url("./icon.svg");
}
@keyframes spin { to { opacity: 0; } }
"#;

fn extractions() -> usize {
    EXTRACTIONS.with(std::cell::Cell::get)
}

fn checked_load(path: &Path, source: &str, mode: CacheMode) -> WireStyle {
    let (facts, warning) = load(path, source.as_bytes(), source, true, mode).unwrap();
    assert!(warning.is_none(), "{warning:?}");
    assert!(facts.backing().is_mapped());
    let view = Syntax::new(facts.facts());
    assert_eq!(facts.facts().urls().len(), 2);
    assert_eq!(
        view.compositions
            .iter()
            .next()
            .unwrap()
            .names
            .iter()
            .next()
            .unwrap()
            .decoded,
        "base"
    );
    assert!(view
        .animations
        .iter()
        .any(|item| item.definition && item.name.decoded == "spin"));
    let allocation = facts.backing().as_ptr() as usize
        ..facts.backing().as_ptr() as usize + facts.backing().len();
    assert!(
        allocation.contains(&(view.names.iter().next().unwrap().name.decoded.as_ptr() as usize))
    );
    facts
}

#[test]
fn stylesheet_facts_persist_and_warm_readonly_hits_never_extract() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("style.module.css");
    fs::write(&path, CSS).unwrap();
    let before = extractions();
    let cold = checked_load(&path, CSS, CacheMode::ReadWrite);
    assert_eq!(extractions(), before + 1);
    let persisted = fs::read(sidecar(&path)).unwrap();
    for mode in [CacheMode::ReadWrite, CacheMode::ReadOnly] {
        let warm = checked_load(&path, CSS, mode);
        assert_eq!(extractions(), before + 1);
        assert_eq!(&**warm.backing(), &**cold.backing());
    }
    assert_eq!(fs::read(sidecar(&path)).unwrap(), persisted);
    let pointer = cold.facts().names().get(0).name().decoded().as_ptr();
    let clone = cold.clone();
    fs::write(sidecar(&path), "replaced while owned").unwrap();
    drop(cold);
    assert_eq!(
        pointer,
        clone.facts().names().get(0).name().decoded().as_ptr()
    );
    assert_eq!(clone.facts().names().get(0).name().decoded(), "theme");
    assert!(segment_cache::is_cache_artifact(&sidecar(&path)));
    assert!(segment_cache::is_cache_artifact(
        &path.with_file_name("style.module.css.style.seg.fb.stage-orphan")
    ));
}

#[test]
fn stylesheet_readonly_misses_disabled_and_failed_writes_stay_mapped() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("style.module.css");
    fs::write(&path, CSS).unwrap();
    let before = extractions();
    checked_load(&path, CSS, CacheMode::ReadOnly);
    assert!(!sidecar(&path).exists());
    fs::write(sidecar(&path), "do not read or overwrite").unwrap();
    checked_load(&path, CSS, CacheMode::Disabled);
    assert_eq!(
        fs::read_to_string(sidecar(&path)).unwrap(),
        "do not read or overwrite"
    );
    fs::remove_file(sidecar(&path)).unwrap();
    fs::create_dir(sidecar(&path)).unwrap();
    let (facts, warning) = load(&path, CSS.as_bytes(), CSS, true, CacheMode::ReadWrite).unwrap();
    assert!(facts.backing().is_mapped());
    assert!(warning.unwrap().contains("publishing stylesheet facts"));
    assert_eq!(extractions(), before + 3);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn stylesheet_corruption_source_and_selection_changes_reextract_without_heap_fallback() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("style.module.css");
    fs::write(&path, CSS).unwrap();
    checked_load(&path, CSS, CacheMode::ReadWrite);
    let original = fs::read(sidecar(&path)).unwrap();
    let before = extractions();
    for bytes in [Vec::new(), vec![0; 16], {
        let mut bytes = original.clone();
        bytes[12] ^= 1;
        bytes
    }] {
        fs::write(sidecar(&path), &bytes).unwrap();
        checked_load(&path, CSS, CacheMode::ReadOnly);
        assert_eq!(fs::read(sidecar(&path)).unwrap(), bytes);
    }
    assert_eq!(extractions(), before + 3);
    fs::write(sidecar(&path), original).unwrap();
    let changed = CSS.replace("opacity: 0", "opacity: 1");
    fs::write(&path, &changed).unwrap();
    checked_load(&path, &changed, CacheMode::ReadWrite);
    assert_eq!(extractions(), before + 4);
    let (global, _) = load(
        &path,
        changed.as_bytes(),
        &changed,
        false,
        CacheMode::ReadWrite,
    )
    .unwrap();
    assert!(global.backing().is_mapped());
    assert!(global.facts().names().is_empty());
    assert_eq!(extractions(), before + 5);
}

#[test]
fn stylesheet_facts_seal_original_and_prepared_sass_independently() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("style.module.scss");
    let input = "$color: red; .button {color: $color}";
    let prepared = ".button {color: red}";
    fs::write(&path, input).unwrap();
    let (record, _) = load(
        &path,
        input.as_bytes(),
        prepared,
        true,
        CacheMode::ReadWrite,
    )
    .unwrap();
    let before = extractions();
    let (warm, _) = load(&path, input.as_bytes(), prepared, true, CacheMode::ReadOnly).unwrap();
    assert_eq!(extractions(), before);
    assert_eq!(&**record.backing(), &**warm.backing());
    let (changed, _) = load(
        &path,
        input.as_bytes(),
        ".button {color: blue}",
        true,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(extractions(), before + 1);
    assert!(changed.backing().is_mapped());
    assert!(WireStyle::from_backing(
        record.backing().clone(),
        b"changed original",
        prepared,
        config(true)
    )
    .is_err());
}

#[test]
fn warm_stylesheet_facts_reject_changed_original_before_returning() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("style.module.css");
    fs::write(&path, CSS).unwrap();
    checked_load(&path, CSS, CacheMode::ReadWrite);
    let before = extractions();
    fs::write(&path, ".changed {}").unwrap();
    let error = load(&path, CSS.as_bytes(), CSS, true, CacheMode::ReadOnly).unwrap_err();
    assert!(format!("{error:#}").contains("source changed during build"));
    assert!(format!("{error:#}").contains("style.module.css"));
    assert_eq!(extractions(), before);
}
