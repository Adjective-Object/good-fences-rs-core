use super::*;

#[test]
fn literal_and_context_resolution_run_off_the_coordinator_even_with_one_worker() {
    use std::sync::{Arc, Mutex};
    struct Observer;
    impl Drop for Observer {
        fn drop(&mut self) {
            resolver::REQUEST_OBSERVER.with(|observer| *observer.borrow_mut() = None);
        }
    }
    let _observer = Observer;
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import './literal.mjs'; import './style.css'; export const load = name => import(`./pages/${name}.mjs`);");
    write(dir.root(), "literal.mjs", "export const value = 1;");
    write(dir.root(), "pages/a.mjs", "export const value = 2;");
    write(dir.root(), "style.css", "@import './child.css';");
    write(dir.root(), "child.css", ".a { color: red }");
    let coordinator = std::thread::current().id();
    for workers in [1, 2, 4] {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let requests = observed.clone();
        resolver::REQUEST_OBSERVER.with(|observer| {
            *observer.borrow_mut() = Some(Arc::new(move || {
                requests.lock().unwrap().push(std::thread::current().id());
            }));
        });
        let mut config = options();
        config.execution =
            crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(workers).unwrap());
        BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        let threads = observed.lock().unwrap();
        assert!(!threads.is_empty());
        assert!(
            threads.iter().all(|thread| *thread != coordinator),
            "{threads:?}"
        );
    }
}

#[test]
fn speculative_inventory_shares_reads_without_authorizing_unused_observations() {
    let dir = TmpDir::new();
    write(dir.root(), "unused.mjs", "export const value = 1;");
    let inventory = Inventory::new(dir.root());
    let speculative = inventory.fork();
    let path = dir.root().join("unused.mjs");
    speculative.admit(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(
        inventory.snapshot().capture(&path).unwrap().kind(),
        ftree_cache::inputs::InputKind::File
    );
    assert!(inventory.observations().is_empty());
    inventory.recheck().unwrap();
    assert!(speculative.recheck().is_err());
    inventory
        .merge_observations(speculative.observations())
        .unwrap();
    assert!(inventory.recheck().is_err());
}

#[test]
fn precise_demand_reuses_a_still_running_speculative_preparation() {
    for workers in [2, 4] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import './dep.mjs'; import './node_modules/signal.mjs';",
        );
        write(dir.root(), "dep.mjs", "export const value = 42;");
        write(dir.root(), "node_modules/signal.mjs", "");
        let (started, start_wait) = std::sync::mpsc::channel();
        let (release, release_wait) = std::sync::mpsc::channel();
        let start_wait = std::sync::Mutex::new(start_wait);
        let release_wait = std::sync::Mutex::new(release_wait);
        let calls = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let observed = calls.clone();
        let observer = observe_preparation(move |path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            *observed.lock().unwrap().entry(name.clone()).or_insert(0) += 1;
            match name.as_str() {
                "main.mjs" => {
                    start_wait
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .expect("eager dependency must start before precise discovery");
                }
                "dep.mjs" => {
                    started.send(()).unwrap();
                    release_wait
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .expect("precise frontier must continue while an eager result is pending");
                }
                "signal.mjs" => {
                    release.send(()).unwrap();
                }
                _ => {}
            }
        });
        let mut config = options();
        config.cache = CacheMode::Disabled;
        config.execution =
            crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(workers).unwrap());
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        drop(observer);
        assert_eq!(result.counters.source_parses, 3);
        assert_eq!(result.counters.speculative_parses, 0);
        assert!(calls.lock().unwrap().values().all(|count| *count == 1));
    }
}

#[test]
fn eager_errors_and_observations_stay_speculative_until_demanded() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(dir.root(), "main.mjs", "export { value } from './dep.mjs';");
    write(dir.root(), "dep.mjs", "export const value = 42;");
    write(dir.root(), "unused-broken.mjs", "export const =");
    write(dir.root(), "unused-import.mjs", "import './missing.mjs';");
    write(dir.root(), "node_modules/never/never.mjs", "invalid");
    let (finished, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    let root = dir.root().to_owned();
    let observer =
        observe_preparation(
            move |path| match path.file_name().unwrap().to_str().unwrap() {
                "main.mjs" => {
                    for _ in 0..2 {
                        wait.lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(30))
                            .expect("speculative sources should run alongside precise work");
                    }
                    fs::remove_file(root.join("unused-broken.mjs")).unwrap();
                }
                "unused-broken.mjs" | "unused-import.mjs" => {
                    finished.send(()).unwrap();
                }
                "never.mjs" => panic!("must not walk node_modules"),
                _ => {}
            },
        );
    let mut config = options();
    config.cache = CacheMode::Disabled;
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(4).unwrap());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = session.build(&config).unwrap();
    drop(observer);
    assert_eq!(built.counters.source_parses, 2);
    assert!(built.counters.speculative_parses > 0);
    assert_eq!(built.counters.source_reparses, 0);
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    assert_eq!(session.sources.len(), 2);
    write(
        dir.root(),
        "unused-import.mjs",
        "import './another-missing.mjs';",
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&built.product, &noop.product));
    write(dir.root(), "unused-broken.mjs", "export const =");
    write(dir.root(), "main.mjs", "import './unused-broken.mjs';");
    assert!(session.build(&config).is_err());
    write(dir.root(), "main.mjs", "import './unused-import.mjs';");
    let error = session.build(&config).err().unwrap();
    assert!(
        format!("{error:#}").contains("another-missing"),
        "{error:#}"
    );
}
