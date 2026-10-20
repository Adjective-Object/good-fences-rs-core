use super::*;
use crate::execution::{ExecutionOptions, JOB_OBSERVER};
use std::num::NonZeroUsize;
use std::sync::{Arc, Barrier, Mutex};

fn emit_repeated(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    sources: &BTreeMap<PathBuf, String>,
) -> Artifacts {
    let mut previous = None;
    for workers in [1, 2, 4, 4, 2, 1] {
        let before = crate::rewrite::parse_counts();
        let preparations = native::global_preparation_count();
        let output = esm_with_execution(
            graph,
            plan,
            &EffectPolicy::default(),
            sources,
            ExecutionOptions::new(NonZeroUsize::new(workers).unwrap()),
        )
        .unwrap();
        assert_eq!(native::global_preparation_count() - preparations, 1);
        let counts = crate::rewrite::parse_counts().difference(before);
        assert_eq!(counts.source, 0, "worker source reparses");
        let executable = output
            .files
            .keys()
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext == "mjs" || ext == "cjs" || ext == "js")
            })
            .count();
        let expected = if cfg!(debug_assertions) {
            executable
        } else {
            0
        };
        assert_eq!(counts.output, expected, "worker fragment/final parses");
        assert_eq!(counts.validation, expected);
        if let Some(previous) = &previous {
            assert_eq!(&output, previous, "worker count {workers}");
        }
        previous = Some(output);
    }
    previous.unwrap()
}

fn entry_plan(graph: &SourceGraph, entries: &[&str]) -> ChunkPlan {
    partition(
        graph,
        &entries
            .iter()
            .map(|entry| {
                let file = graph.file_id(Path::new(entry)).unwrap();
                RootSeed {
                    id: RootId::Entry((*entry).into()),
                    evaluation_roots: vec![file],
                    binding_roots: vec![GraphNode::Module(file)],
                }
            })
            .collect::<Vec<_>>(),
        &EffectPolicy::default(),
    )
    .unwrap()
}

#[test]
fn parallel_owners_preserve_bytes_collisions_mutation_activation_and_async_order() {
    let sources = sources(&[
        ("a.mjs", "export let value = 1; export function bump(){value++} console.log('a');"),
        ("b.mjs", "export const value = 10; console.log('b');"),
        ("private.mjs", "const __gf_default_0 = 7; export default __gf_default_0;"),
        ("late.cjs", "console.log('late'); module.exports = {value: 30};"),
        ("loader.cjs", "exports.load = () => require('./late.cjs');"),
        ("async.mjs", "console.log('async-start'); await 0; console.log('async-end'); export default 40;"),
        ("entry.mjs", "import {value, bump} from './a.mjs'; import {value as other} from './b.mjs'; import privateValue from './private.mjs'; import loader from './loader.cjs'; const __gf_import_0 = 5, __gf_default_0 = 6; console.log('entry', value, other, privateValue, __gf_import_0, __gf_default_0); bump(); console.log('mutated', value); console.log('loaded', loader.load().value, loader.load().value); console.log('dynamic', (await import('./async.mjs')).default);"),
    ]);
    let graph = graph(&sources, false);
    let plan = entry_plan(&graph, &["entry.mjs", "a.mjs", "b.mjs", "loader.cjs"]);
    let output = emit_repeated(&graph, &plan, &sources);
    let baseline = node(&sources, Path::new("entry.mjs"));
    let observed = node(&output.files, &output.entries["entry.mjs"]);
    for result in [&baseline, &observed] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(observed.stdout, baseline.stdout);
    assert_eq!(observed.stdout, b"a\nb\nentry 1 10 7 5 6\nmutated 2\nlate\nloaded 30 30\nasync-start\nasync-end\ndynamic 40\n");
    let cjs = node(&output.files, &output.entries["loader.cjs"]);
    assert!(
        cjs.status.success(),
        "{}",
        String::from_utf8_lossy(&cjs.stderr)
    );
    assert!(
        cjs.stdout.is_empty(),
        "availability must not activate late.cjs"
    );
}

#[test]
fn parallel_owners_preserve_native_tdz() {
    let sources = sources(&[
        ("entry.mjs", "import {read} from './cycle.mjs'; export let value = 1; console.log(read());"),
        ("cycle.mjs", "import {value} from './entry.mjs'; console.log(value); export function read(){ return value; }"),
    ]);
    let graph = graph(&sources, false);
    let output = emit_repeated(
        &graph,
        &entry_plan(&graph, &["entry.mjs", "cycle.mjs"]),
        &sources,
    );
    for result in [
        node(&sources, Path::new("entry.mjs")),
        node(&output.files, &output.entries["entry.mjs"]),
    ] {
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("ReferenceError"));
    }
}

#[test]
fn parallel_async_siblings_preserve_native_evaluation_interleaving() {
    let sources = sources(&[
        (
            "left.mjs",
            "console.log('left-start'); await 0; console.log('left-end');",
        ),
        (
            "right.mjs",
            "console.log('right-start'); await 0; console.log('right-end');",
        ),
        (
            "entry.mjs",
            "import './left.mjs'; import './right.mjs'; console.log('entry');",
        ),
    ]);
    let graph = graph(&sources, false);
    let output = emit_repeated(&graph, &entry_plan(&graph, &["entry.mjs"]), &sources);
    let baseline = node(&sources, Path::new("entry.mjs"));
    let observed = node(&output.files, &output.entries["entry.mjs"]);
    for result in [&baseline, &observed] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(observed.stdout, baseline.stdout);
    assert_eq!(
        observed.stdout,
        b"left-start\nright-start\nleft-end\nright-end\nentry\n"
    );
}

#[test]
fn parallel_relocated_bindings_keep_cross_owner_addresses_and_global_exclusions() {
    let alias = format!("__gf_segment_import_{}_8", digest("entry.mjs"));
    let sources = sources(&[
        ("shared.mjs", "export let state = 0; export function bump(){state++} export const left = () => state; export const right = () => state; console.log('shared');"),
        ("default.mjs", "export default (() => 2);"),
        ("blocker.mjs", &format!("export const __gf_default_0 = 10; export {{__gf_default_0 as __gf_default_1}}; export function read({alias}_) {{ return [{alias}_, typeof {alias}, typeof __gf_default_2]; }}")),
        ("entry.mjs", "import {left, state, bump} from './shared.mjs'; import value from './default.mjs'; import {read} from './blocker.mjs'; const right = 9; bump(); console.log(left(), state, right, value.name, value(), read(5).join(',')); export {left, state, bump};"),
        ("other.mjs", "import {right, state, bump} from './shared.mjs'; export {right, state, bump};"),
    ]);
    let baseline = node(&sources, Path::new("entry.mjs"));
    assert!(baseline.status.success());
    let mut previous = None;
    for reverse in [false, true] {
        let graph = graph(&sources, reverse);
        let output = emit_repeated(
            &graph,
            &entry_plan(&graph, &["entry.mjs", "other.mjs", "blocker.mjs"]),
            &sources,
        );
        let state_owner = &output.members[&NodeId::Segment {
            module: "shared.mjs".into(),
            index: 0,
        }];
        assert_ne!(state_owner, &output.entries["entry.mjs"]);
        assert_ne!(state_owner, &output.entries["other.mjs"]);
        let observed = node(&output.files, &output.entries["entry.mjs"]);
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert_eq!(observed.stdout, baseline.stdout);
        if let Some(previous) = previous {
            assert_eq!(output, previous);
        }
        previous = Some(output);
    }
}

#[test]
fn parallel_commonjs_owner_groups_keep_cycles_cache_and_snapshot_order() {
    let sources = sources(&[
        ("a.cjs", "console.log('a'); exports.value = 1; const b = require('./b.cjs'); exports.read = () => [exports.value, b.value]; exports.bump = () => exports.value++;"),
        ("b.cjs", "console.log('b'); exports.value = 2; exports.a = require('./a.cjs');"),
        ("c.cjs", "console.log('c'); module.exports = {value: 3};"),
        ("d.cjs", "console.log('d'); module.exports = {value: 4};"),
        ("entry.mjs", "import a, {value} from './a.cjs'; import * as b from './b.cjs'; console.log(a.read().join(','), value, b.default.a === a); a.bump(); console.log(a.read().join(','), value);"),
    ]);
    let graph = graph(&sources, false);
    let output = emit_repeated(
        &graph,
        &entry_plan(&graph, &["entry.mjs", "a.cjs", "b.cjs", "c.cjs", "d.cjs"]),
        &sources,
    );
    for entry in ["entry.mjs", "a.cjs", "b.cjs", "c.cjs", "d.cjs"] {
        let baseline = node(&sources, Path::new(entry));
        let observed = node(&output.files, &output.entries[entry]);
        for result in [&baseline, &observed] {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(observed.stdout, baseline.stdout);
    }
}

#[test]
fn parallel_selected_and_multi_owner_low_level_roots_are_identical() {
    let sources = sources(&[
        ("a.mjs", "export const value = 1; console.log('a');"),
        ("b.mjs", "export const value = 2; console.log('b');"),
        ("entry.mjs", "import {value as a} from './a.mjs'; import {value as b} from './b.mjs'; console.log(a+b);"),
    ]);
    let graph = graph(&sources, false);
    let a = graph.file_id(Path::new("a.mjs")).unwrap();
    let b = graph.file_id(Path::new("b.mjs")).unwrap();
    let selected = graph
        .resolve_export(a, "value", source_graph::Namespace::Value)
        .unwrap();
    let plan = partition(
        &graph,
        &[
            RootSeed {
                id: RootId::Entry("selected".into()),
                evaluation_roots: vec![],
                binding_roots: selected.route,
            },
            RootSeed {
                id: RootId::Entry("multi".into()),
                evaluation_roots: vec![b, a],
                binding_roots: vec![],
            },
        ],
        &EffectPolicy::default(),
    )
    .unwrap();
    let output = emit_repeated(&graph, &plan, &sources);
    let observed = node(&output.files, &output.entries["multi"]);
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, b"b\na\n");
}

#[test]
fn parallel_real_owner_jobs_rendezvous_without_timing_assertions() {
    let sources = sources(&[
        ("a.mjs", "export const value = 1;"),
        ("b.mjs", "export const value = 2;"),
        ("c.mjs", "export const value = 3;"),
        ("d.mjs", "export const value = 4;"),
    ]);
    let graph = graph(&sources, false);
    let plan = entry_plan(&graph, &["a.mjs", "b.mjs", "c.mjs", "d.mjs"]);
    for workers in [2, 4] {
        let barrier = Arc::new(Barrier::new(workers));
        let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let observed = threads.clone();
        JOB_OBSERVER.with(|observer| {
            *observer.borrow_mut() = Some(Arc::new(move |count, index| {
                if count >= workers && index < workers {
                    observed.lock().unwrap().insert(std::thread::current().id());
                    barrier.wait();
                }
            }))
        });
        let result = esm_with_execution(
            &graph,
            &plan,
            &EffectPolicy::default(),
            &sources,
            ExecutionOptions::new(NonZeroUsize::new(workers).unwrap()),
        );
        JOB_OBSERVER.with(|observer| *observer.borrow_mut() = None);
        result.unwrap();
        assert_eq!(threads.lock().unwrap().len(), workers);
    }
}
