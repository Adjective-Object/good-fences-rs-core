use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ast_segmenter::{parse_module, ModuleFormat};
use bundler::{
    chunks::{partition, ChunkPlan, NodeId, RootId, RootSeed},
    policy::EffectPolicy,
};
use source_graph::{DependencyTarget, OwnedSourceFileInput, SourceGraph};

struct MeasuredAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static MEASURE: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static REALLOCS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn allocated(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    if MEASURE.load(Ordering::Relaxed) {
        PEAK.fetch_max(live, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
            if MEASURE.load(Ordering::Relaxed) {
                ALLOCS.fetch_add(1, Ordering::Relaxed);
            }
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        if MEASURE.load(Ordering::Relaxed) {
            DEALLOCS.fetch_add(1, Ordering::Relaxed);
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            if size >= layout.size() {
                allocated(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
            if MEASURE.load(Ordering::Relaxed) {
                REALLOCS.fetch_add(1, Ordering::Relaxed);
            }
        }
        pointer
    }
}

fn input(path: &str, source: &str) -> OwnedSourceFileInput {
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        ModuleFormat::Esm,
    )
    .unwrap();
    let resolutions = parsed
        .facts()
        .dependencies()
        .iter()
        .map(|dependency| {
            (
                dependency.id(),
                DependencyTarget::File(dependency.specifier().into()),
            )
        })
        .collect();
    OwnedSourceFileInput::from_parsed(path.into(), parsed, resolutions)
}

fn fixture(roots: usize, closure: usize) -> (SourceGraph, Vec<RootSeed>, Vec<PathBuf>, String) {
    let prefix = "long-application-package-component\\".repeat(8);
    let paths: Vec<_> = (0..closure)
        .map(|index| format!("{prefix}shared-{index:04}.mjs"))
        .collect();
    let name = "requested_export_with_a_long_original_name_".repeat(4);
    let request = paths[0].replace('\\', "\\\\");
    let source = format!("import('{request}').then(ns => ns.{name});").repeat(roots);
    let mut inputs = vec![input(&format!("{prefix}entry.mjs"), &source)];
    for (index, path) in paths.iter().enumerate() {
        let request = paths[(index + 1) % closure].replace('\\', "\\\\");
        let export = if index == 0 {
            format!("export const {name} = 7;")
        } else {
            String::new()
        };
        inputs.push(input(
            path,
            &format!("import '{request}'; globalThis.observe(); {export}"),
        ));
    }
    inputs.push(input("dead.mjs", "throw new Error('unreachable');"));
    let graph = SourceGraph::from_owned(inputs);
    graph.validate_links().unwrap();
    let entries = vec![RootSeed {
        id: RootId::Entry("main".into()),
        evaluation_roots: vec![0],
        binding_roots: vec![],
    }];
    (
        graph,
        entries,
        paths.into_iter().map(PathBuf::from).collect(),
        name,
    )
}

fn assert_semantics(plan: &ChunkPlan, roots: usize, paths: &[PathBuf], name: &str) {
    let selected: Vec<_> = plan
        .roots
        .keys()
        .filter(|root| matches!(root, RootId::AsyncExport { .. }))
        .cloned()
        .collect();
    assert_eq!(selected.len(), roots);
    assert_eq!(plan.async_activations.len(), roots);
    for root in &selected {
        let RootId::AsyncExport { site, name: actual } = root else {
            unreachable!()
        };
        assert_eq!(actual, name);
        assert_eq!(
            plan.async_activations[site].roots,
            BTreeSet::from([root.clone()])
        );
    }
    for path in paths {
        let mask = &plan.node_roots[&NodeId::Module(path.clone())];
        assert!(mask.iter().eq(selected.iter()));
        assert!(!mask.contains(&RootId::Entry("main".into())));
    }
    assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());

    // Independent owned inversion of the public forward projection, after measurement.
    let mut expected = BTreeMap::<_, BTreeSet<_>>::new();
    for (root, nodes) in &plan.reachability {
        for node in nodes {
            expected
                .entry(node.clone())
                .or_default()
                .insert(root.clone());
        }
    }
    let mut chunks = BTreeMap::<_, Vec<_>>::new();
    for (node, mask) in &plan.node_roots {
        let owned = mask.to_owned_set();
        assert_eq!(owned, expected.get(node).cloned().unwrap_or_default());
        assert!(mask.iter().rev().eq(owned.iter().rev()));
        assert_eq!(mask.iter().len(), owned.len());
        if !owned.is_empty() {
            chunks.entry(owned).or_default().push(node.clone());
        }
    }
    let actual: BTreeMap<_, _> = plan
        .chunks
        .iter()
        .map(|(chunk, nodes)| (chunk.0.to_owned_set(), nodes.clone()))
        .collect();
    assert_eq!(actual, chunks);
}

#[test]
fn overlapping_async_export_roots_keep_partition_storage_compact() {
    // A dedicated executable with one test isolates global requested-byte accounting.
    // Graph construction/validation is outside the region; the returned plan stays live.
    for roots in [24, 96] {
        for closure in [32, 128] {
            let (graph, entries, paths, name) = fixture(roots, closure);
            let policy = EffectPolicy::default();
            let baseline = LIVE.load(Ordering::Relaxed);
            PEAK.store(baseline, Ordering::Relaxed);
            ALLOCS.store(0, Ordering::Relaxed);
            REALLOCS.store(0, Ordering::Relaxed);
            DEALLOCS.store(0, Ordering::Relaxed);
            MEASURE.store(true, Ordering::Relaxed);
            let plan = partition(&graph, &entries, &policy).unwrap();
            let held = LIVE.load(Ordering::Relaxed);
            let peak = PEAK.load(Ordering::Relaxed);
            MEASURE.store(false, Ordering::Relaxed);
            let retained = held.saturating_sub(baseline);
            let memberships: usize = plan.reachability.values().map(|nodes| nodes.len()).sum();
            assert_semantics(&plan, roots, &paths, &name);
            drop(plan);
            // Read before printing: libtest's captured-output buffer is retained too.
            let after_drop = LIVE.load(Ordering::Relaxed) as i128 - baseline as i128;
            eprintln!(
                "root_mask_memory roots={roots} closure={closure} memberships={memberships} retained_requested_bytes={retained} peak_requested_bytes={} allocs={} reallocs={} deallocs={}",
                peak.saturating_sub(baseline),
                ALLOCS.load(Ordering::Relaxed),
                REALLOCS.load(Ordering::Relaxed),
                DEALLOCS.load(Ordering::Relaxed),
            );
            // Scalar forward membership plus per-node/root metadata; no path payload
            // or tree entry may be retained for every inverted membership.
            let bound = 512 * 1024 + closure * 4096 + roots * 8192 + memberships * 16;
            assert!(
                retained < bound,
                "partition retained {retained} requested bytes, bound {bound}; roots={roots}, closure={closure}"
            );

            eprintln!(
                "root_mask_memory roots={roots} closure={closure} after_drop_bytes={after_drop}"
            );
            assert_eq!(after_drop, 0, "partition storage must return to baseline");
        }
    }
}
