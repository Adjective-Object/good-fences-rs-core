use super::*;
use test_tmpdir::TmpDir;

fn write(root: &Path, name: &str, text: &str) {
    fs::write(root.join(name), text).unwrap();
}

fn fixture(root: &Path) {
    write(root, "package.json", r#"{"type":"module"}"#);
    write(root, "main.mjs", "export {};");
    write(root, "dep.ts", "export const value = 1;");
    write(root, "other.mjs", "export const value = 2;");
}

fn resolver(root: &Path, options: &BuildOptions, memory: &Memory) -> BuildResolver {
    let lockfile = Lockfile::read(root, &mut Vec::new(), options.execution).unwrap();
    BuildResolver::new(root, options, lockfile, memory.clone())
}

#[test]
fn repeated_requests_share_one_current_row_publication_index_and_retained_memo() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = BuildOptions::default();
    let memory = Memory::default();
    let resolver = resolver(dir.root(), &config, &memory);
    let base = dir.root().join("main.mjs");
    let first = resolver
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    resolver
                        .resolve_request(&base, "./dep", ResolutionMode::Import)
                        .unwrap()
                })
            })
            .collect();
        for handle in handles {
            assert!(Arc::ptr_eq(&first, &handle.join().unwrap()));
        }
    });
    assert_eq!(resolver.requests.read().len(), 1);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert!(memory.0.read().is_empty());
    {
        let packages = resolver.packages.read();
        let fresh = &packages[dir.root()].fresh;
        assert_eq!(fresh.len(), 1);
        assert!(Arc::ptr_eq(&fresh[0], &first));
    }
    resolver.demand(&first).unwrap();
    resolver.demand(&first).unwrap();
    let memory = memory.0.read();
    let (key, retained) = memory.first_key_value().unwrap();
    assert_eq!(memory.len(), 1);
    assert!(Arc::ptr_eq(key, &first.memo.key));
    assert!(Arc::ptr_eq(retained, &first.memo));
}

#[test]
fn early_resolution_failure_retains_only_demanded_requests_for_retry() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = BuildOptions {
        cache: CacheMode::ReadOnly,
        ..Default::default()
    };
    let memory = Memory::default();
    let current = resolver(dir.root(), &config, &memory);
    let base = dir.root().join("main.mjs");
    let speculative = current
        .resolve_request(&base, "./other.mjs", ResolutionMode::Import)
        .unwrap();
    let unused = Arc::downgrade(&speculative.memo);
    drop(speculative);
    let precise = current
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    current.demand(&precise).unwrap();
    assert!(current
        .resolve_request(&base, "./missing", ResolutionMode::Import)
        .is_err());
    assert_eq!(current.requests.read().len(), 2);
    assert_eq!(memory.0.read().len(), 1);
    drop(current);
    assert!(unused.upgrade().is_none());

    let retry = resolver(dir.root(), &config, &memory);
    let reused = retry
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(reused.cache_hit);
    assert!(Arc::ptr_eq(&precise.memo, &reused.memo));
    let unretained = retry
        .resolve_request(&base, "./other.mjs", ResolutionMode::Import)
        .unwrap();
    assert!(!unretained.cache_hit);
    retry.demand(&reused).unwrap();
    retry.retain_precise();
    assert_eq!(retry.requests.read().len(), 1);
    assert!(retry.packages.read().values().all(|package| package
        .fresh
        .iter()
        .all(|request| request.demanded.load(Ordering::Relaxed))));
}

#[test]
fn retained_requests_revalidate_new_epochs_and_separate_policy_keys() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = BuildOptions {
        cache: CacheMode::ReadOnly,
        ..Default::default()
    };
    let memory = Memory::default();
    let base = dir.root().join("main.mjs");
    let cold = resolver(dir.root(), &config, &memory);
    let original = cold
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    cold.demand(&original).unwrap();
    drop(cold);
    let warm = resolver(dir.root(), &config, &memory);
    let hit = warm
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(hit.cache_hit);
    assert!(Arc::ptr_eq(&original.memo, &hit.memo));
    // Repeated requests in one epoch reuse the checked row, not retained lookup.
    write(dir.root(), "dep.js", "export const value = 3;");
    assert!(Arc::ptr_eq(
        &hit,
        &warm
            .resolve_request(&base, "./dep", ResolutionMode::Import)
            .unwrap()
    ));
    // A direct resolver rechecks every current row; production first prunes
    // speculative rows with retain_precise().
    assert!(warm.recheck().is_err());
    warm.demand(&hit).unwrap();
    assert!(warm.recheck().is_err());
    drop(warm);

    let next = resolver(dir.root(), &config, &memory);
    let shadowed = next
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(!shadowed.cache_hit);
    assert_eq!(
        shadowed.memo.target,
        ResolvedDependency::File(dir.root().join("dep.js"))
    );
    next.demand(&shadowed).unwrap();
    let mut other_policy = config.clone();
    other_policy
        .resolver
        .alias
        .insert("./dep".into(), "./other.mjs".into());
    let other = resolver(dir.root(), &other_policy, &memory);
    let aliased = other
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(!aliased.cache_hit);
    assert_ne!(aliased.memo.key.0, shadowed.memo.key.0);
    assert_eq!(
        aliased.memo.target,
        ResolvedDependency::File(dir.root().join("other.mjs"))
    );
    other.demand(&aliased).unwrap();
    assert_eq!(memory.0.read().len(), 2);
}

#[test]
fn disk_hits_share_original_borrowed_proofs_with_retained_memory() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let config = BuildOptions::default();
    let base = dir.root().join("main.mjs");
    let cold = resolver(dir.root(), &config, &Memory::default());
    let fresh = cold
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    cold.resolve_request(&base, "./other.mjs", ResolutionMode::Import)
        .unwrap();
    cold.demand(&fresh).unwrap();
    cold.retain_precise();
    cold.recheck().unwrap();
    cold.finish(&mut Counters::default(), &mut Vec::new());
    drop(cold);

    let memory = Memory::default();
    let disk = resolver(dir.root(), &config, &memory);
    let loaded = disk
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    {
        let packages = disk.packages.read();
        let record = packages[dir.root()].loaded.as_ref().unwrap();
        assert!(record
            .find(wire::Mode(0), "main.mjs", "./other.mjs")
            .is_none());
    }
    assert!(loaded.cache_hit);
    assert!(matches!(loaded.memo.proof, Proof::Disk(..)));
    disk.demand(&loaded).unwrap();
    assert!(disk
        .packages
        .read()
        .values()
        .all(|package| package.fresh.is_empty()));
    drop(disk);
    let warm = resolver(dir.root(), &config, &memory);
    let retained = warm
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(retained.cache_hit);
    assert!(Arc::ptr_eq(&loaded.memo, &retained.memo));
    assert_eq!(
        retained
            .memo
            .file_routes(&dir.root().join("dep.ts"))
            .unwrap(),
        loaded.memo.file_routes(&dir.root().join("dep.ts")).unwrap()
    );
}
