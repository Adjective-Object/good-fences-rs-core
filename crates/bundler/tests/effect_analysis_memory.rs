use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ast_segmenter::{parse_module_with_prepared_declarations, ModuleFormat, PreparedDeclarations};
use bundler::{
    chunks::{partition, RootId, RootSeed},
    policy::EffectPolicy,
};
use segment_cache::CacheRecord;
use source_graph::{DependencyTarget, OwnedSourceFileInput, SourceGraph};

struct MeasuredAllocator;

static MEASURE: AtomicBool = AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

fn record(size: usize) {
    if MEASURE.load(Ordering::Relaxed) {
        LARGEST.fetch_max(size, Ordering::Relaxed);
    }
}

fn graph(
    count: usize,
    commonjs: bool,
    borrowed: bool,
    declarations: &PreparedDeclarations,
) -> SourceGraph {
    let mut inputs = Vec::new();
    for index in 0..count {
        let path = format!("module-{index}.mjs");
        let source = if index == 0 && commonjs {
            "import './legacy.cjs'; export const answer = Math.abs(-42);"
        } else {
            "export const answer = Math.abs(-42);"
        };
        let parsed = parse_module_with_prepared_declarations(
            &logger::StdioLogger::new(),
            source,
            Path::new(&path),
            ModuleFormat::Esm,
            declarations,
        )
        .unwrap();
        let parsed = if borrowed {
            CacheRecord::from_parsed(source, &parsed)
                .unwrap()
                .to_parsed(source)
                .unwrap()
        } else {
            parsed
        };
        let resolutions = parsed
            .facts()
            .dependencies()
            .iter()
            .map(|dependency| (dependency.id(), DependencyTarget::File("legacy.cjs".into())))
            .collect();
        inputs.push(OwnedSourceFileInput::from_parsed(
            path.into(),
            parsed,
            resolutions,
        ));
    }
    if commonjs {
        let parsed = parse_module_with_prepared_declarations(
            &logger::StdioLogger::new(),
            "module.exports = 7;",
            Path::new("legacy.cjs"),
            ModuleFormat::CommonJs,
            declarations,
        )
        .unwrap();
        inputs.push(OwnedSourceFileInput::from_parsed(
            "legacy.cjs".into(),
            parsed,
            Default::default(),
        ));
    }
    SourceGraph::from_owned(inputs)
}

#[test]
fn conservative_effect_fallbacks_do_not_instantiate_unused_solver() {
    let declarations = PreparedDeclarations::new(
        ecco::DeclarationConfig::default()
            .effective_for_environment(ecco::Environment::Browser)
            .unwrap(),
    )
    .unwrap();
    let entries = [RootSeed {
        id: RootId::Entry("main".into()),
        evaluation_roots: vec![0],
        binding_roots: vec![],
    }];
    for commonjs in [true, false] {
        let graph = graph(64, commonjs, !commonjs, &declarations);
        let policy = EffectPolicy::default().with_pure_annotations(commonjs);
        LARGEST.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        let result = partition(&graph, &entries, &policy);
        MEASURE.store(false, Ordering::Relaxed);
        let plan = result.unwrap();
        assert_eq!(plan.retention.modules.contains(&64), commonjs);
        assert!(!plan.retention.modules.contains(&1));
        assert!(plan
            .retention
            .segments
            .iter()
            .any(|segment| segment.key.file_id == 0));
        let largest = LARGEST.load(Ordering::Relaxed);
        eprintln!("commonjs={commonjs}: largest planning allocation: {largest}");
        assert!(
            largest < 1024 * 1024,
            "conservative planning allocated a {largest}-byte block for unused inferred effects"
        );
    }

    let eligible = graph(1, false, true, &declarations);
    let plan = partition(&eligible, &entries, &EffectPolicy::default()).unwrap();
    assert!(
        plan.retention.segments.is_empty(),
        "eligible pure ESM still uses inferred pruning"
    );
}
