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

#[test]
fn context_initialization_is_single_flight_and_handles_do_not_lock_shards() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    struct Counted;
    impl ContextData for Counted {
        fn read_context_data(_: (), _: &Path) -> anyhow::Result<Option<Self>> {
            COUNT.fetch_add(1, Ordering::Relaxed);
            Ok(Some(Self))
        }
    }
    let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
    let cache = Arc::new(FileContextCache::<Counted, "context">::new());
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| assert!(cache.check_dir(dir.root()).unwrap().is_some()));
        }
    });
    assert_eq!(COUNT.load(Ordering::Relaxed), 1);
    let held = cache.check_dir(dir.root()).unwrap();
    cache.mark_dirty_root(dir.root());
    assert!(held.is_some());
    assert!(cache.check_dir(dir.root()).unwrap().is_some());
    assert_eq!(COUNT.load(Ordering::Relaxed), 2);
}

#[test]
fn context_initializers_can_lookup_other_contexts_without_a_shard_lock() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Nested;
    type Cache = FileContextCache<Nested, "context">;
    thread_local! {
        static CURRENT: std::cell::RefCell<Option<std::sync::Arc<Cache>>> = const { std::cell::RefCell::new(None) };
    }
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    impl ContextData for Nested {
        fn read_context_data(_: (), path: &Path) -> anyhow::Result<Option<Self>> {
            if COUNT.fetch_add(1, Ordering::Relaxed) == 0 {
                CURRENT.with(|cache| {
                    let cache = cache.borrow();
                    for i in 0..128 {
                        assert!(cache
                            .as_ref()
                            .unwrap()
                            .check_dir(&path.parent().unwrap().join(format!("other-{i}")))?
                            .is_some());
                    }
                    Ok::<_, anyhow::Error>(())
                })?;
            }
            Ok(Some(Self))
        }
    }
    let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
    let cache = std::sync::Arc::new(Cache::new());
    CURRENT.with(|current| *current.borrow_mut() = Some(cache.clone()));
    assert!(cache.check_dir(dir.root()).unwrap().is_some());
    CURRENT.with(|current| *current.borrow_mut() = None);
    assert_eq!(COUNT.load(Ordering::Relaxed), 129);
}

#[test]
fn cached_context_errors_retain_the_path_and_typed_io_cause() {
    struct Failed;
    impl ContextData for Failed {
        fn read_context_data(_: (), _: &Path) -> anyhow::Result<Option<Self>> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "reader denied").into())
        }
    }
    let dir = test_tmpdir::test_tmpdir! { "entry" => "" };
    let cache = FileContextCache::<Failed, "context">::new();
    for _ in 0..2 {
        let observer = InputObserver::default();
        let error = cache
            .check_dir_with_inputs(dir.root(), Some(&observer))
            .err()
            .unwrap();
        assert!(error.chain().any(|cause| cause.is::<std::io::Error>()));
        let context = dir.root().join("context").display().to_string();
        assert!(format!("{error:#}").contains(&context));
        assert!(format!("{:#}", observer.proof().check_errors().unwrap_err()).contains(&context));
    }
}
