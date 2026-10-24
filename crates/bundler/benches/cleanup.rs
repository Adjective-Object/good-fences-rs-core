use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

use bundler::{
    build::{BuildOptions, BuildSession, Entry},
    execution::ExecutionOptions,
};
use test_tmpdir::TmpDir;

struct CountingAllocator;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static DEALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static FREED_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATIONS.fetch_add(1, Relaxed);
        FREED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            DEALLOCATIONS.fetch_add(1, Relaxed);
            ALLOCATED_BYTES.fetch_add(size as u64, Relaxed);
            FREED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn snapshot() -> [u64; 4] {
    [
        ALLOCATIONS.load(Relaxed),
        DEALLOCATIONS.load(Relaxed),
        ALLOCATED_BYTES.load(Relaxed),
        FREED_BYTES.load(Relaxed),
    ]
}

fn measure<R>(name: &str, work: impl FnOnce() -> R) -> R {
    let before = snapshot();
    let start = Instant::now();
    let result = work();
    let elapsed = start.elapsed();
    let after = snapshot();
    println!(
        "{name}: ms={:.3} allocations={} deallocations={} allocated_bytes={} freed_bytes={}",
        elapsed.as_secs_f64() * 1000.0,
        after[0] - before[0],
        after[1] - before[1],
        after[2] - before[2],
        after[3] - before[3],
    );
    result
}

fn main() -> eyre::Result<()> {
    const MODULES: usize = 128;
    for workers in [1, 4] {
        let dir = TmpDir::new();
        let root = dir.root();
        fs::write(root.join("package.json"), r#"{"type":"module"}"#)?;
        fs::write(root.join("shared.mjs"), "export const shared = 1;")?;
        let mut entry = String::new();
        for i in 0..MODULES {
            let name = format!("m{i}.mjs");
            fs::write(
                root.join(&name),
                format!("import {{shared}} from './shared.mjs'; export const v{i} = shared + {i};"),
            )?;
            entry.push_str(&format!("export {{v{i}}} from './{name}';\n"));
        }
        fs::write(root.join("entry.mjs"), &entry)?;
        let options = BuildOptions {
            entries: BTreeMap::from([(
                "entry".into(),
                Entry {
                    path: "entry.mjs".into(),
                    output_name: None,
                    namespace: true,
                },
            )]),
            execution: ExecutionOptions::new(workers.try_into().unwrap()),
            ..BuildOptions::default()
        };
        println!("workers={workers} modules={}", MODULES + 2);
        let mut session = BuildSession::new(root)?;
        let result = measure("cold_build", || session.build(&options))?;
        let noop = measure("noop_build", || session.build(&options))?;
        assert!(std::rc::Rc::ptr_eq(&result.product, &noop.product));
        drop(noop);
        fs::write(
            root.join("entry.mjs"),
            format!("{entry}\nimport './missing.mjs';"),
        )?;
        let error = measure("failed_attempt_including_preparation", || {
            session.build(&options)
        })
        .err()
        .ok_or_else(|| eyre::eyre!("expected missing import to fail"))?;
        measure("error_drop", || drop(error));
        measure("session_drop_with_product_alive", || drop(session));
        assert!(result
            .product
            .artifacts
            .files
            .values()
            .any(|s| s.contains("v0")));
        measure("last_product_drop", || drop(result));
        let mut failure_options = options.clone();
        failure_options.cache = segment_cache::CacheMode::Disabled;
        let mut failed_session = BuildSession::new(root)?;
        let error = measure("uncached_failed_attempt_including_preparation", || {
            failed_session.build(&failure_options)
        })
        .err()
        .ok_or_else(|| eyre::eyre!("expected uncached missing import to fail"))?;
        measure("uncached_error_drop", || drop(error));
        measure("failed_session_drop_without_product", || {
            drop(failed_session)
        });
    }
    Ok(())
}
