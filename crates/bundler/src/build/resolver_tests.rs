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

fn resolver<'s, 'a>(
    root: &Path,
    options: &BuildOptions,
    memory: &Memory,
    scratch: &'s Scratch<'a>,
) -> BuildResolver<'s, 'a> {
    let lockfile = Lockfile::read(root, &mut Vec::new(), options.execution).unwrap();
    BuildResolver::new(root, options, lockfile, memory.clone(), scratch)
}

#[test]
fn failed_optional_metadata_resolution_rechecks_additions_without_persisting_a_target() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let config = BuildOptions::default();
    let memory = Memory::default();
    let current = resolver(dir.root(), &config, &memory, &scratch);
    assert!(current
        .resolve_request(
            &dir.root().join("main.mjs"),
            "./unused.mts",
            ResolutionMode::Import
        )
        .is_err());
    assert!(current.failed_inputs.read().is_empty());
    assert!(current
        .resolve_request_with_failures(
            &dir.root().join("main.mjs"),
            "./missing.mts",
            ResolutionMode::Import,
            true,
        )
        .is_err());
    current.recheck().unwrap();
    write(dir.root(), "unused.mts", "export {};");
    current.recheck().unwrap();
    assert!(current.requests.read().is_empty());
    assert!(memory.0.read().is_empty());
    write(dir.root(), "missing.mts", "export interface Shape {}");
    let error = current.recheck().unwrap_err();
    assert!(
        format!("{error:#}").contains("failed optional resolution inputs"),
        "{error:#}"
    );
}

#[test]
fn repeated_requests_share_one_current_row_publication_index_and_retained_memo() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let config = BuildOptions::default();
    let memory = Memory::default();
    let resolver = resolver(dir.root(), &config, &memory, &scratch);
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
    let (key, retained) = memory.iter().next().unwrap();
    assert_eq!(memory.len(), 1);
    assert_eq!(key.as_ref(), &first.row.key.owned());
    assert!(Arc::ptr_eq(&retained.proof, &first.proof));
}

#[test]
fn early_resolution_failure_retains_only_demanded_requests_for_retry() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let config = BuildOptions {
        cache: CacheMode::ReadOnly,
        ..Default::default()
    };
    let memory = Memory::default();
    let current = resolver(dir.root(), &config, &memory, &scratch);
    let base = dir.root().join("main.mjs");
    let speculative = current
        .resolve_request(&base, "./other.mjs", ResolutionMode::Import)
        .unwrap();
    let unused = Arc::downgrade(&speculative.proof);
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

    let retry = resolver(dir.root(), &config, &memory, &scratch);
    let reused = retry
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(reused.cache_hit);
    assert!(Arc::ptr_eq(&precise.proof, &reused.proof));
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
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let config = BuildOptions {
        cache: CacheMode::ReadOnly,
        ..Default::default()
    };
    let memory = Memory::default();
    let base = dir.root().join("main.mjs");
    let cold = resolver(dir.root(), &config, &memory, &scratch);
    let original = cold
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    cold.demand(&original).unwrap();
    drop(cold);
    let warm = resolver(dir.root(), &config, &memory, &scratch);
    let hit = warm
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(hit.cache_hit);
    assert!(Arc::ptr_eq(&original.proof, &hit.proof));
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

    let next = resolver(dir.root(), &config, &memory, &scratch);
    let shadowed = next
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(!shadowed.cache_hit);
    assert_eq!(
        shadowed.row.target.owned(),
        ResolvedDependency::File(dir.root().join("dep.js"))
    );
    next.demand(&shadowed).unwrap();
    let mut other_policy = config.clone();
    other_policy
        .resolver
        .alias
        .insert("./dep".into(), "./other.mjs".into());
    let other = resolver(dir.root(), &other_policy, &memory, &scratch);
    let aliased = other
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(!aliased.cache_hit);
    assert_ne!(aliased.row.key.policy, shadowed.row.key.policy);
    assert_eq!(
        aliased.row.target.owned(),
        ResolvedDependency::File(dir.root().join("other.mjs"))
    );
    other.demand(&aliased).unwrap();
    assert_eq!(memory.0.read().len(), 2);
}

#[test]
fn disk_hits_share_original_borrowed_proofs_with_retained_memory() {
    let dir = TmpDir::new();
    fixture(dir.root());
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let config = BuildOptions::default();
    let base = dir.root().join("main.mjs");
    let cold = resolver(dir.root(), &config, &Memory::default(), &scratch);
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
    let disk = resolver(dir.root(), &config, &memory, &scratch);
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
    assert!(matches!(loaded.proof.as_ref(), Proof::Disk(..)));
    disk.demand(&loaded).unwrap();
    assert!(disk
        .packages
        .read()
        .values()
        .all(|package| package.fresh.is_empty()));
    drop(disk);
    let warm = resolver(dir.root(), &config, &memory, &scratch);
    let retained = warm
        .resolve_request(&base, "./dep", ResolutionMode::Import)
        .unwrap();
    assert!(retained.cache_hit);
    assert!(Arc::ptr_eq(&loaded.proof, &retained.proof));
    assert_eq!(
        retained.file_routes(&dir.root().join("dep.ts")).unwrap(),
        loaded.file_routes(&dir.root().join("dep.ts")).unwrap()
    );
}

#[test]
fn failed_and_panicked_attempts_drop_scratch_and_speculative_owners_but_keep_precise_proofs() {
    for workers in [1, 4] {
        let dir = TmpDir::new();
        fixture(dir.root());
        let config = BuildOptions {
            cache: CacheMode::ReadOnly,
            ..Default::default()
        };
        let memory = Memory::default();
        let base = dir.root().join("main.mjs");
        let mut unused = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let herd = bumpalo_herd::Herd::new();
            let scratch = Scratch::new(&herd);
            let current = resolver(dir.root(), &config, &memory, &scratch);
            let speculative = current
                .resolve_request(&base, "./other.mjs", ResolutionMode::Import)
                .unwrap();
            unused = Some(Arc::downgrade(&speculative.proof));
            drop(speculative);
            let barrier = std::sync::Barrier::new(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..workers)
                    .map(|_| {
                        let (current, base, barrier) = (&current, &base, &barrier);
                        scope.spawn(move || {
                            barrier.wait();
                            let request = current
                                .resolve_request(base, "./dep", ResolutionMode::Import)
                                .unwrap();
                            current.demand(&request).unwrap();
                        })
                    })
                    .collect();
                for handle in handles {
                    handle.join().unwrap();
                }
            });
            assert!(current
                .resolve_request(&base, "./missing", ResolutionMode::Import)
                .is_err());
            panic!("attempt cleanup");
        }));
        assert!(result.is_err());
        assert!(unused.unwrap().upgrade().is_none());
        assert_eq!(memory.0.read().len(), 1);
        let herd = bumpalo_herd::Herd::new();
        let scratch = Scratch::new(&herd);
        let retry = resolver(dir.root(), &config, &memory, &scratch);
        let precise = retry
            .resolve_request(&base, "./dep", ResolutionMode::Import)
            .unwrap();
        assert!(precise.cache_hit);
        retry.demand(&precise).unwrap();
        retry.recheck().unwrap();
        write(dir.root(), "dep.js", "export const value = 3;");
        assert!(retry.recheck().is_err());
    }
}

#[test]
fn scratch_key_lookups_preserve_path_equivalence_and_compare_full_identity_on_hash_collisions() {
    let dir = TmpDir::new();
    let herd = bumpalo_herd::Herd::new();
    let scratch = Scratch::new(&herd);
    let package = Key {
        policy: [1; 32],
        instance: [2; 32],
        dependencies: [3; 32],
    };
    let base = dir.root().join("main.mjs");
    let normalized = dir.root().join(".").join("main.mjs");
    let query = Query::new(package, &base, 0, "first");
    let key = scratch.alloc(query.allocate(&scratch));
    let mut other = Query::new(package, &base, 0, "second");
    other.hash = query.hash;
    let mut table = hashbrown::HashMap::new();
    table.insert(key, 1);
    table.insert(scratch.alloc(other.allocate(&scratch)), 2);
    assert_eq!(table.get(&query), Some(&1));
    assert_eq!(
        table.get(&Query::new(package, &normalized, 0, "first")),
        Some(&1)
    );
    assert_eq!(table.get(&other), Some(&2));
    assert_eq!(table.len(), 2);
    let mut paths = [
        dir.root().join("a-").join("main.mjs"),
        dir.root().join("a").join("main.mjs"),
        dir.root().join("\u{e000}").join("main.mjs"),
        dir.root().join("\u{10000}").join("main.mjs"),
    ];
    let mut keys: Vec<_> = paths
        .iter()
        .map(|path| Query::new(package, path, 0, "first").allocate(&scratch))
        .collect();
    keys.sort_by_cached_key(|key| key.sort_key());
    paths.sort();
    assert_eq!(
        keys.iter()
            .map(|key| key.base.to_path_buf())
            .collect::<Vec<_>>(),
        paths
    );
}
