use crate::{
    context_data::{ContextData, FileContextCache},
    inputs::InputObserver,
};
use std::path::Path;

#[test]
fn context_hits_replay_original_negative_inputs_until_invalidated() {
    let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
    let cache = FileContextCache::<(), "context">::new();
    assert!(cache.check_dir(dir.root()).unwrap().is_none());
    std::fs::write(dir.root().join("context"), "").unwrap();
    let observer = InputObserver::default();
    assert!(cache
        .check_dir_with_inputs(dir.root(), Some(&observer))
        .unwrap()
        .is_none());
    assert!(observer.proof().validate().is_err());
    cache.mark_dirty_root(dir.root());
    let observer = InputObserver::default();
    assert!(cache
        .check_dir_with_inputs(dir.root(), Some(&observer))
        .unwrap()
        .is_some());
    observer.proof().validate().unwrap();
}

#[test]
fn unproven_prepopulated_and_legacy_contexts_cannot_authorize_tracked_hits() {
    struct Legacy;
    impl ContextData for Legacy {
        fn read_context_data(_: (), _: &Path) -> anyhow::Result<Option<Self>> {
            Ok(Some(Self))
        }
    }
    let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
    let cache = FileContextCache::<Legacy, "context">::new();
    assert!(cache.check_dir(dir.root()).unwrap().is_some());
    let observer = InputObserver::default();
    assert!(cache
        .check_dir_with_inputs(dir.root(), Some(&observer))
        .unwrap()
        .is_some());
    assert!(observer.proof().check_errors().is_err());

    let cache = FileContextCache::<(), "context">::new();
    cache.prepopulate(dir.root(), ());
    let observer = InputObserver::default();
    assert!(cache
        .check_dir_with_inputs(dir.root(), Some(&observer))
        .unwrap()
        .is_some());
    assert!(observer.proof().check_errors().is_err());
}
