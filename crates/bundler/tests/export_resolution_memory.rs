use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashSet,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ast_segmenter::{parse_module, ModuleFormat};
use source_graph::{
    DependencyTarget, ExportIdentity, GraphNode, Namespace, OwnedSourceFileInput, SegmentKey,
    SourceGraph,
};

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

fn diamond(depth: usize) -> SourceGraph {
    let mut inputs = vec![input(
        "root.mjs",
        "export * from 'left-0.mjs'; export * from 'right-0.mjs';",
    )];
    for level in 0..depth {
        let source = if level + 1 == depth {
            "export * from 'leaf.mjs';".to_owned()
        } else {
            format!(
                "export * from 'left-{}.mjs'; export * from 'right-{}.mjs';",
                level + 1,
                level + 1
            )
        };
        for side in ["left", "right"] {
            inputs.push(input(&format!("{side}-{level}.mjs"), &source));
        }
    }
    inputs.push(input("leaf.mjs", "export const value = 7;"));
    SourceGraph::from_owned(inputs)
}

// The known fixture topology supplies an independent first-visit route oracle.
fn expected_route(depth: usize) -> Vec<GraphNode> {
    fn visit(
        file: u32,
        level: usize,
        depth: usize,
        seen: &mut HashSet<GraphNode>,
        route: &mut Vec<GraphNode>,
    ) {
        let leaf = (2 * depth + 1) as u32;
        let children = if file == leaf {
            vec![]
        } else if level == depth {
            vec![leaf]
        } else {
            vec![(2 * level + 1) as u32, (2 * level + 2) as u32]
        };
        for index in 0..children.len().max(1) {
            let node = GraphNode::Segment(SegmentKey::new(file, index as u32));
            if seen.insert(node) {
                route.push(node);
            }
            if let Some(&child) = children.get(index) {
                visit(child, level + 1, depth, seen, route);
            }
        }
    }
    let mut route = Vec::new();
    visit(0, 0, depth, &mut HashSet::new(), &mut route);
    route
}

#[test]
fn layered_diamonds_preserve_checked_identity_and_routes() {
    // One test in its own executable isolates the process-wide measurement.
    for depth in [8, 12] {
        let graph = diamond(depth);
        graph.validate_links().unwrap();
        drop(graph.resolve_export(0, "value", Namespace::Value).unwrap());
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        ALLOCS.store(0, Ordering::Relaxed);
        REALLOCS.store(0, Ordering::Relaxed);
        DEALLOCS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        let linked = graph.resolve_export(0, "value", Namespace::Value).unwrap();
        let held = LIVE.load(Ordering::Relaxed);
        let peak = PEAK.load(Ordering::Relaxed);
        drop(linked);
        let after_drop = LIVE.load(Ordering::Relaxed);
        MEASURE.store(false, Ordering::Relaxed);
        eprintln!(
            "export_resolution_memory depth={depth} files={} live_result_bytes={} peak_requested_bytes={} after_drop_bytes={} allocs={} reallocs={} deallocs={}",
            graph.file_count(),
            held as i128 - baseline as i128,
            peak.saturating_sub(baseline),
            after_drop as i128 - baseline as i128,
            ALLOCS.load(Ordering::Relaxed),
            REALLOCS.load(Ordering::Relaxed),
            DEALLOCS.load(Ordering::Relaxed),
        );

        let linked = graph.resolve_export(0, "value", Namespace::Value).unwrap();
        let leaf = (2 * depth + 1) as u32;
        let binding = graph
            .file_facts(leaf)
            .unwrap()
            .bindings()
            .get(0)
            .unwrap()
            .id();
        assert_eq!(linked.identity, ExportIdentity::Binding(leaf, binding));
        assert_eq!(
            linked.declaration,
            GraphNode::Segment(SegmentKey::new(leaf, 0))
        );
        assert_eq!(linked.route, expected_route(depth));
    }
}
