use super::*;
use crate::chunks::{partition, RootSeed};
use ast_segmenter::{parse_module, ModuleFormat};
use source_graph::OwnedSourceFileInput;
use std::path::{Component, Path};
use std::process::{Command, Output};
use test_tmpdir::TmpDir;

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                assert!(result.pop());
            }
            other => result.push(other),
        }
    }
    result
}

fn graph(sources: &BTreeMap<PathBuf, String>, reverse: bool) -> SourceGraph {
    graph_with_format(sources, reverse, ModuleFormat::Esm)
}

fn graph_with_format(
    sources: &BTreeMap<PathBuf, String>,
    reverse: bool,
    format: ModuleFormat,
) -> SourceGraph {
    let mut inputs: Vec<_> = sources
        .iter()
        .map(|(path, source)| {
            let parsed = parse_module(&logger::StdioLogger::new(), source, path, format).unwrap();
            let resolutions = parsed
                .facts
                .dependencies
                .iter()
                .map(|d| {
                    let target = normalize(&path.parent().unwrap().join(&d.specifier));
                    let target = if sources.contains_key(&target) {
                        target
                    } else {
                        ["js", "cjs", "mjs"]
                            .iter()
                            .map(|ext| target.with_extension(ext))
                            .find(|path| sources.contains_key(path))
                            .unwrap_or(target)
                    };
                    (d.id, DependencyTarget::File(target))
                })
                .collect();
            OwnedSourceFileInput::from_parsed(path.clone(), parsed, resolutions)
        })
        .collect();
    if reverse {
        inputs.reverse();
    }
    SourceGraph::from_owned(inputs)
}

fn sources(items: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
    items
        .iter()
        .map(|(p, s)| (PathBuf::from(p), (*s).into()))
        .collect()
}

fn build(
    sources: &BTreeMap<PathBuf, String>,
    entries: &[(&str, bool)],
    reverse: bool,
) -> (ChunkPlan, Artifacts) {
    build_with_format(sources, entries, reverse, ModuleFormat::Esm)
}

fn build_with_format(
    sources: &BTreeMap<PathBuf, String>,
    entries: &[(&str, bool)],
    reverse: bool,
    format: ModuleFormat,
) -> (ChunkPlan, Artifacts) {
    let graph = graph_with_format(sources, reverse, format);
    let roots: Vec<_> = entries
        .iter()
        .map(|(entry, namespace)| {
            let file = graph.file_id(Path::new(entry)).unwrap();
            RootSeed {
                id: RootId::Entry((*entry).into()),
                evaluation_roots: vec![file],
                binding_roots: if *namespace {
                    vec![GraphNode::Module(file)]
                } else {
                    vec![]
                },
            }
        })
        .collect();
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    let output = esm(&graph, &plan, &policy, sources).unwrap();
    (plan, output)
}

fn canonical(
    sources: &BTreeMap<PathBuf, String>,
    reverse: bool,
) -> (SourceGraph, ChunkPlan, crate::symbols::Symbols) {
    let graph = graph(sources, reverse);
    let file = graph.file_id(Path::new("entry.mjs")).unwrap();
    let plan = partition(
        &graph,
        &[RootSeed {
            id: RootId::Entry("entry".into()),
            evaluation_roots: vec![file],
            binding_roots: vec![GraphNode::Module(file)],
        }],
        &EffectPolicy::default(),
    )
    .unwrap();
    let symbols = crate::symbols::Symbols::link(&graph, &plan).unwrap();
    (graph, plan, symbols)
}

fn deferred(sources: &BTreeMap<PathBuf, String>) -> Option<DeferredEsm> {
    let (graph, plan, _) = canonical(sources, false);
    let parses = ast_segmenter::module_parse_count();
    let output = deferred_esm(&graph, &plan, &EffectPolicy::default(), sources).unwrap();
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    output
}

fn observe_deferred(mut output: DeferredEsm, observer: &str) {
    let entry = &output.artifacts.entries["entry"];
    output.artifacts.files.insert(
        PathBuf::from("observe.mjs"),
        format!(
            "import assert from 'node:assert/strict';\n\
             globalThis.trace = [];\n\
             globalThis.makeSecret = () => {{ trace.push('secret'); return {{}}; }};\n\
             const ns = await import('./{}');\n\
             const initName = {};\n\
             const init = ns[initName];\n{}",
            entry.display(),
            quote(&output.initializer),
            observer,
        ),
    );
    let actual = node(&output.artifacts.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
}

#[test]
fn deferred_esm_keeps_root_captures_live_bindings_and_function_identity() {
    let input = sources(&[(
        "entry.mjs",
        "const secret = makeSecret();\n\
         export let count = 0;\n\
         export function read() { return secret; }\n\
         export function increment() { return ++count; }\n\
         trace.push(this === undefined);",
    )]);
    let output = deferred(&input).expect("synchronous binding island");
    observe_deferred(
        output,
        "assert.deepEqual(trace, []);\n\
         const read = ns.read;\n\
         assert.equal(read(), undefined);\n\
         ns[initName]();\n\
         const secret = read();\n\
         assert.equal(typeof secret, 'object');\n\
         assert.equal(ns.read, read);\n\
         assert.equal(ns.count, 0);\n\
         assert.equal(ns.increment(), 1);\n\
         init(); init();\n\
         assert.equal(ns.count, 1);\n\
         assert.equal(read(), secret);\n\
         assert.deepEqual(trace, ['secret', true]);",
    );
}

#[test]
fn deferred_esm_caches_every_thrown_value_without_retry() {
    for thrown in [
        "undefined",
        "null",
        "false",
        "0",
        "'failure'",
        "{reason: 1}",
    ] {
        let input = sources(&[(
            "entry.mjs",
            "trace.push('entered'); export let partial = {}; throw globalThis.thrown;",
        )]);
        observe_deferred(
            deferred(&input).unwrap(),
            &format!(
                "globalThis.thrown = {thrown};\n\
                 assert.deepEqual(trace, []);\n\
                 for (let attempt = 0; attempt < 3; attempt++) {{\n\
                   let caught = false;\n\
                   try {{ init(); }} catch (error) {{\n\
                     caught = true; assert.equal(error, globalThis.thrown);\n\
                   }}\n\
                   assert.equal(caught, true);\n\
                 }}\n\
                 assert.equal(typeof ns.partial, 'object');\n\
                 assert.deepEqual(trace, ['entered']);"
            ),
        );
    }
}

#[test]
fn deferred_esm_reentry_is_rejected_without_repeating_effects() {
    let input = sources(&[(
        "entry.mjs",
        "trace.push('entered'); reenter(); export const value = 1;",
    )]);
    observe_deferred(
        deferred(&input).unwrap(),
        "globalThis.reenter = init;\n\
         const rejected = error => error === 'Unsupported deferred ESM reentry';\n\
         assert.throws(init, rejected);\n\
         assert.throws(init, rejected);\n\
         assert.deepEqual(trace, ['entered']);",
    );
}

#[test]
fn deferred_esm_reconstructs_split_declarations_and_static_dependency_order() {
    let input = sources(&[
        ("entry.mjs",
         "import {dep} from './z-dependency.mjs';\n\
          export const first = (trace.push('first'), dep), second = (trace.push('second'), first);\n\
          export function read() { return second; }\n\
          trace.push('entry');"),
        ("z-dependency.mjs", "export const dep = (trace.push('dependency'), {});"),
    ]);
    observe_deferred(
        deferred(&input).unwrap(),
        "assert.deepEqual(trace, []);\n\
         init();\n\
         assert.equal(ns.first, ns.second);\n\
         assert.equal(ns.read(), ns.first);\n\
         init();\n\
         assert.deepEqual(trace, ['dependency', 'first', 'second', 'entry']);",
    );
}

#[test]
fn deferred_esm_names_and_declaration_kinds_preserve_existing_restrictions() {
    let input = sources(&[(
        "entry.mjs",
        "export const __gf_init = makeSecret();\n\
         const __gf_init_failure = 3;\n\
         export const arrow = () => __gf_init;\n\
         export function read() { return __gf_init_failure; }\n\
         export var value; value = 4; var value;\n\
         export let empty;\n\
         trace.push(typeof __gf_init_state, typeof __gf_init_caught);",
    )]);
    observe_deferred(
        deferred(&input).unwrap(),
        "assert.deepEqual(trace, []);\n\
         init();\n\
         const arrow = ns.arrow;\n\
         assert.equal(arrow.name, 'arrow');\n\
         assert.equal(arrow(), ns.__gf_init);\n\
         assert.equal(ns.read.name, 'read');\n\
         assert.equal(ns.read(), 3);\n\
         assert.equal(ns.value, 4);\n\
         assert.equal(ns.empty, undefined);\n\
         init();\n\
         assert.equal(ns.arrow, arrow);\n\
         assert.deepEqual(trace, ['secret', 'undefined', 'undefined']);",
    );
}

#[test]
fn deferred_esm_unsupported_shapes_keep_native_evaluation() {
    for source in [
        "export const {x} = {x: 1};",
        "export class Thing {}",
        "export const value = await Promise.resolve(1);",
        "export const value = 1; export function write() { value = 2; }",
        "if (true) { var value = 1; } export {value};",
        "export default function () {}",
    ] {
        let input = sources(&[("entry.mjs", source)]);
        assert!(deferred(&input).is_none(), "{source}");
        let (_, native) = build(&input, &[("entry.mjs", true)], false);
        assert!(!native
            .files
            .values()
            .any(|code| code.contains("Unsupported deferred ESM reentry")));
    }

    for input in [
        sources(&[
            ("entry.mjs", "import './dep.mjs'; export const value = 1;"),
            ("dep.mjs", "import './entry.mjs';"),
        ]),
        sources(&[
            (
                "entry.mjs",
                "import * as ns from './dep.mjs'; export const value = ns;",
            ),
            ("dep.mjs", "export const x = 1;"),
        ]),
        sources(&[
            (
                "entry.mjs",
                "import {x} from './dep.mjs'; x = 2; export {x};",
            ),
            ("dep.mjs", "export let x = 1;"),
        ]),
    ] {
        assert!(deferred(&input).is_none());
    }
}

#[test]
fn deferred_esm_rejects_repaired_source_and_stale_policy() {
    let input = sources(&[("entry.mjs", "export const value = 1;")]);
    let (graph, plan, _) = canonical(&input, false);
    let changed = sources(&[("entry.mjs", "export const value = 2;")]);
    assert!(deferred_esm(&graph, &plan, &EffectPolicy::default(), &changed).is_err());
    let changed_policy = EffectPolicy::parse(["makeSecret(...)"]).unwrap();
    assert!(deferred_esm(&graph, &plan, &changed_policy, &input).is_err());
}

fn activated(
    input: &BTreeMap<PathBuf, String>,
    placements: &[(&str, &str)],
) -> Artifacts<crate::assembly::Code> {
    let (graph, plan, _) = canonical(input, false);
    let placements = placements
        .iter()
        .map(|(file, owner)| {
            (
                graph.file_id(Path::new(file)).unwrap(),
                graph.file_id(Path::new(owner)).unwrap(),
            )
        })
        .collect();
    let parses = ast_segmenter::module_parse_count();
    let output = esm_plan_activated(
        &graph,
        &plan,
        &EffectPolicy::default(),
        input,
        &BTreeMap::new(),
        &placements,
    )
    .unwrap();
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    output
}

fn observe_activated(mut output: Artifacts, observer: &str) {
    output.files.insert(
        PathBuf::from("observe.mjs"),
        format!(
            "import assert from 'node:assert/strict';\n\
         globalThis.trace = [];\n\
         const entry = await import({});\n{}",
            quote(&format!("./{}", output.entries["entry"].display())),
            observer,
        ),
    );
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
}

fn automatically_hoisted(input: &BTreeMap<PathBuf, String>) -> Artifacts {
    let (graph, plan, _) = canonical(input, false);
    let parses = ast_segmenter::module_parse_count();
    let output = esm(&graph, &plan, &EffectPolicy::default(), input).unwrap();
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    output
}

fn clustered(input: &BTreeMap<PathBuf, String>, target: usize) -> (ChunkPlan, Artifacts) {
    let (graph, plan, _) = canonical(input, false);
    let parses = ast_segmenter::module_parse_count();
    let output = esm_with_options(
        &graph,
        &plan,
        &EffectPolicy::default(),
        input,
        &crate::build::OutputOptions {
            min_chunk_bytes: target,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    (plan, output)
}

fn library_input(failure: bool) -> BTreeMap<PathBuf, String> {
    let mut input = sources(&[
        (
            "entry.mjs",
            "export async function a() { return (await import('./root-a.mjs')).read(); } \
          export async function b() { return (await import('./root-b.mjs')).read(); } \
          export async function c() { return (await import('./root-c.mjs')).read(); }",
        ),
        (
            "lib-x.mjs",
            "trace.push('x'); const secret = {}; export let count = 0; \
          export function readX() { count++; return [secret, readX, count]; }",
        ),
        (
            "lib-y.mjs",
            if failure {
                "trace.push('y'); throw undefined; export function readY() { return 7; }"
            } else {
                "trace.push('y'); export function readY() { return 7; }"
            },
        ),
    ]);
    for (file, deps, body) in [
        (
            "root-a.mjs",
            "import {readX} from './lib-x.mjs';",
            "return readX();",
        ),
        (
            "root-b.mjs",
            "import {readX} from './lib-x.mjs'; import {readY} from './lib-y.mjs';",
            "return [readX(), readY()];",
        ),
        (
            "root-c.mjs",
            "import {readY} from './lib-y.mjs';",
            "return readY();",
        ),
    ] {
        input.insert(
            file.into(),
            format!(
                "{deps} export function read() {{ if (false) return '{}'; {body} }}",
                "padding".repeat(400)
            ),
        );
    }
    input
}

fn module_owner<'a>(output: &'a Artifacts, file: &str) -> &'a PathBuf {
    &output.members[&NodeId::Module(file.into())]
}

#[test]
fn libraries_merge_different_overlapping_masks_without_coactivation_or_binding_copies() {
    let input = library_input(false);
    let (plan, output) = clustered(&input, 1024);
    let x = NodeId::Module("lib-x.mjs".into());
    let y = NodeId::Module("lib-y.mjs".into());
    assert_ne!(plan.node_roots[&x], plan.node_roots[&y]);
    assert_eq!(
        plan.node_roots[&x]
            .intersection(&plan.node_roots[&y])
            .count(),
        1
    );
    assert_eq!(output.members[&x], output.members[&y]);
    assert_ne!(
        module_owner(&output, "lib-x.mjs"),
        module_owner(&output, "root-a.mjs")
    );
    assert_eq!(clustered(&input, 1024).1, output);
    assert_ne!(
        module_owner(&clustered(&input, 0).1, "lib-x.mjs"),
        module_owner(&clustered(&input, 0).1, "lib-y.mjs")
    );
    observe_activated(
        output,
        "assert.deepEqual(trace, []); const first = await entry.a(); \
         assert.deepEqual(trace, ['x']); const again = await entry.a(); \
         assert.equal(first[0],again[0]); assert.equal(first[1],again[1]); \
         const both = await entry.b(); assert.deepEqual(trace,['x','y']); \
         assert.equal(both[0][0],first[0]); assert.equal(both[0][1],first[1]); \
         assert.equal(both[0][2],3); assert.equal(both[1],7); assert.equal(await entry.c(),7);",
    );
}

#[test]
fn libraries_resolve_barrel_redirects_to_final_physical_owners() {
    let mut input = library_input(false);
    input.insert("barrel.mjs".into(), "export * from './lib-x.mjs';".into());
    let root = input.get_mut(Path::new("root-a.mjs")).unwrap();
    *root = root.replace("./lib-x.mjs", "./barrel.mjs");
    let (_, output) = clustered(&input, 1024);
    assert_eq!(
        module_owner(&output, "barrel.mjs"),
        module_owner(&output, "lib-x.mjs")
    );
    assert_eq!(
        module_owner(&output, "lib-x.mjs"),
        module_owner(&output, "lib-y.mjs")
    );
    observe_activated(
        output,
        "assert.deepEqual(trace,[]); const first = await entry.a(); \
         assert.deepEqual(trace,['x']); const both = await entry.b(); \
         assert.equal(first[0],both[0][0]); assert.equal(first[1],both[0][1]); \
         assert.equal(both[0][2],2); assert.equal(both[1],7); \
         assert.equal(await entry.c(),7); assert.deepEqual(trace,['x','y']);",
    );
}

#[test]
fn libraries_private_then_does_not_assimilate_dynamic_availability_imports() {
    let mut input = library_input(false);
    input.insert(
        "entry.mjs".into(),
        "export async function a() { return (await import('./lib-x.mjs')).readX(); } \
         export async function b() { return (await import('./root-b.mjs')).read(); } \
         export async function c() { return (await import('./root-c.mjs')).read(); }"
            .into(),
    );
    input.remove(Path::new("root-a.mjs"));
    input.insert(
        "lib-x.mjs".into(),
        "trace.push('x'); function then(resolve) { resolve(123); } \
         export function readX() { return then; }"
            .into(),
    );
    let (_, output) = clustered(&input, 1024);
    let owner = module_owner(&output, "lib-x.mjs");
    assert_eq!(owner, module_owner(&output, "lib-y.mjs"));
    assert_ne!(owner, &output.entries["entry"]);
    observe_activated(
        output,
        "assert.deepEqual(trace,[]); const privateThen = await entry.a(); \
         assert.equal(typeof privateThen,'function'); let value; privateThen(v => value = v); \
         assert.equal(value,123); assert.deepEqual(trace,['x']); \
         const both = await entry.b(); assert.equal(both[0],privateThen); \
         assert.equal(both[1],7); assert.equal(await entry.c(),7); \
         assert.equal(await entry.a(),privateThen); assert.deepEqual(trace,['x','y']);",
    );
}

#[test]
fn libraries_cache_undefined_failures_across_consumers_and_preserve_dependency_order() {
    let (_, output) = clustered(&library_input(true), 1024);
    assert_eq!(
        module_owner(&output, "lib-x.mjs"),
        module_owner(&output, "lib-y.mjs")
    );
    observe_activated(
        output,
        "assert.deepEqual(trace, []); const reject = error => error === undefined; \
         const [a,b] = await Promise.allSettled([entry.b(),entry.c()]); \
         assert.equal(a.status,'rejected'); assert.equal(b.status,'rejected'); \
         assert.equal(a.reason,undefined); assert.equal(b.reason,undefined); \
         await assert.rejects(entry.b(),reject); await assert.rejects(entry.c(),reject); \
         assert.deepEqual(trace,['x','y']); const live = await entry.a(); \
         assert.equal(live[2],1); assert.deepEqual(trace,['x','y']);",
    );
}

fn bridge_input() -> BTreeMap<PathBuf, String> {
    let mut input = sources(&[
        (
            "entry.mjs",
            "export async function a() { await import('./root-a.mjs'); } \
          export async function b() { await import('./root-b.mjs'); } \
          export async function c() { await import('./root-c.mjs'); } \
          export async function d() { await import('./root-d.mjs'); }",
        ),
        ("a-only.mjs", "trace.push('a');"),
        ("b-bridge.mjs", "trace.push('bridge');"),
        ("c-only.mjs", "trace.push('b');"),
        ("d-disjoint.mjs", "trace.push('disjoint');"),
    ]);
    for (file, deps) in [
        (
            "root-a.mjs",
            "import './a-only.mjs'; import './b-bridge.mjs';",
        ),
        (
            "root-b.mjs",
            "import './b-bridge.mjs'; import './c-only.mjs';",
        ),
        ("root-c.mjs", "import './d-disjoint.mjs';"),
        ("root-d.mjs", "import './d-disjoint.mjs';"),
    ] {
        input.insert(
            file.into(),
            format!("{deps} trace.push('{}');", "padding".repeat(400)),
        );
    }
    input
}

#[test]
fn libraries_update_union_for_bridge_masks_and_leave_disjoint_or_large_islands_out() {
    let input = bridge_input();
    let (plan, output) = clustered(&input, 80);
    let mask = |name: &str| &plan.node_roots[&NodeId::Module(name.into())];
    assert!(mask("a-only.mjs").is_disjoint(mask("c-only.mjs")));
    assert_eq!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "b-bridge.mjs")
    );
    assert_eq!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "c-only.mjs")
    );
    assert_ne!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "d-disjoint.mjs")
    );
    assert_ne!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "root-a.mjs")
    );
    assert_eq!(plan.node_roots, canonical(&input, false).1.node_roots);
}

#[test]
fn libraries_threshold_stopping_overshoot_and_exact_large_cutoff_are_deterministic() {
    let input = bridge_input();
    let first = input[Path::new("a-only.mjs")].len();
    let bridge = input[Path::new("b-bridge.mjs")].len();
    let size = first + bridge;
    for target in [size, size - 1] {
        let (_, output) = clustered(&input, target);
        assert_eq!(
            module_owner(&output, "a-only.mjs"),
            module_owner(&output, "b-bridge.mjs")
        );
        assert_ne!(
            module_owner(&output, "a-only.mjs"),
            module_owner(&output, "c-only.mjs")
        );
        assert!(size < 2 * target);
        assert_eq!(clustered(&input, target).1, output);
    }
    let (_, output) = clustered(&input, bridge);
    assert_ne!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "b-bridge.mjs")
    );
    assert_ne!(
        module_owner(&output, "a-only.mjs"),
        module_owner(&output, "c-only.mjs")
    );
    let (_, larger) = clustered(&input, size + 1);
    assert_eq!(
        module_owner(&larger, "a-only.mjs"),
        module_owner(&larger, "c-only.mjs")
    );
}

#[test]
fn libraries_unsupported_activation_boundaries_remain_native() {
    for unsupported in [
        "await 0; trace.push('y'); export function readY() { return 7; }",
        "export class readY {}",
        "import './root-c.mjs'; trace.push('y'); export function readY() { return 7; }",
    ] {
        let mut input = library_input(false);
        input.insert("lib-y.mjs".into(), unsupported.into());
        let (_, output) = clustered(&input, 1024);
        assert_ne!(
            module_owner(&output, "lib-x.mjs"),
            module_owner(&output, "lib-y.mjs")
        );
    }

    let mut input = library_input(false);
    input.insert(
        "entry.mjs".into(),
        "export function a() { return import('./root-a.mjs'); } \
         export function b() { return import('./root-b.mjs'); } \
         export function c() { return import('./root-c.mjs'); }"
            .into(),
    );
    let (_, output) = clustered(&input, 1024);
    assert_ne!(
        module_owner(&output, "lib-x.mjs"),
        module_owner(&output, "lib-y.mjs")
    );
}

#[test]
fn libraries_cluster_compatible_binding_islands_without_wrapping_ordinary_sources() {
    let mut input = library_input(false);
    input.insert("lib-x.mjs".into(), "export class X {}".into());
    input.insert("lib-y.mjs".into(), "export class Y {}".into());
    for (file, deps, body) in [
        ("root-a.mjs", "import {X} from './lib-x.mjs';", "return X;"),
        (
            "root-b.mjs",
            "import {X} from './lib-x.mjs'; import {Y} from './lib-y.mjs';",
            "return [X,Y];",
        ),
        ("root-c.mjs", "import {Y} from './lib-y.mjs';", "return Y;"),
    ] {
        input.insert(
            file.into(),
            format!(
                "{deps} export function read() {{ if(false) return '{}'; {body} }}",
                "padding".repeat(400)
            ),
        );
    }
    let (_, output) = clustered(&input, 1024);
    let segment = |name: &str| NodeId::Segment {
        module: name.into(),
        index: 0,
    };
    assert_eq!(
        output.members[&segment("lib-x.mjs")],
        output.members[&segment("lib-y.mjs")]
    );
    assert!(!output
        .files
        .values()
        .any(|s| s.contains("Unsupported deferred ESM reentry")));
    observe_activated(
        output,
        "const x = await entry.a(); const y = await entry.c(); const both = await entry.b(); \
         assert.equal(x,both[0]); assert.equal(y,both[1]); assert.equal(x.name,'X'); \
         assert.equal(y.name,'Y'); assert.deepEqual(trace,[]);",
    );
}

#[test]
fn libraries_keep_existing_parent_and_shared_static_initializer_owners() {
    let mut input = library_input(false);
    input.get_mut(Path::new("entry.mjs")).unwrap().push_str(
        " export async function child() { return (await import('./child.mjs')).read(); }",
    );
    input.insert(
        "child.mjs".into(),
        "import './child-dependency.mjs'; trace.push('child'); \
         export function read() { return 9; }"
            .into(),
    );
    input.insert(
        "child-dependency.mjs".into(),
        "trace.push('child-dependency');".into(),
    );
    let (_, output) = clustered(&input, 1024);
    assert_eq!(module_owner(&output, "child.mjs"), &output.entries["entry"]);
    assert_eq!(
        module_owner(&output, "child-dependency.mjs"),
        &output.entries["entry"]
    );
    assert_ne!(module_owner(&output, "lib-x.mjs"), &output.entries["entry"]);
    observe_activated(
        output,
        "assert.deepEqual(trace,[]); await entry.a(); assert.deepEqual(trace,['x']); \
         assert.equal(await entry.child(),9); \
         assert.deepEqual(trace,['x','child-dependency','child']); \
         const both = await entry.b(); assert.equal(both[1],7); \
         assert.deepEqual(trace,['x','child-dependency','child','y']);",
    );
}

#[test]
fn libraries_portable_fixture_really_clusters_different_masks() {
    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!(
                "../../../fixtures/bundler/authored/native-library-clustering/files/",
                $file
            ))
        };
    }
    let input = sources(&[
        ("entry.mjs", fixture!("entry.mjs")),
        ("lib-x.mjs", fixture!("lib-x.mjs")),
        ("lib-y.mjs", fixture!("lib-y.mjs")),
        ("root-a.mjs", fixture!("root-a.mjs")),
        ("root-b.mjs", fixture!("root-b.mjs")),
        ("root-c.mjs", fixture!("root-c.mjs")),
        ("failure.mjs", fixture!("failure.mjs")),
    ]);
    let (plan, output) = clustered(&input, 256);
    assert_ne!(
        plan.node_roots[&NodeId::Module("lib-x.mjs".into())],
        plan.node_roots[&NodeId::Module("lib-y.mjs".into())]
    );
    assert_eq!(
        module_owner(&output, "lib-x.mjs"),
        module_owner(&output, "lib-y.mjs")
    );
    assert_ne!(module_owner(&output, "lib-x.mjs"), &output.entries["entry"]);
}

#[test]
fn single_parent_public_esm_hoists_retained_sites_and_preserves_live_root_identity() {
    let input = sources(&[
        (
            "entry.mjs",
            "export async function load(enabled) { if (!enabled) return false; \
          const ns = await import('./child.mjs'); const saved = ns.read; \
          const before = ns.value; ns.change(); return [before, ns.value, saved(), saved]; } \
          export function other() { return import('./child.mjs').then(ns => ns.value); } \
          function dead() { return import('./child.mjs'); }",
        ),
        (
            "child.mjs",
            "const secret = (trace.push('child'), {}); export let value = 1; \
          export function read() { return secret; } export function change() { value++; }",
        ),
    ]);
    let output = automatically_hoisted(&input);
    assert_eq!(output.files.len(), 1, "{:#?}", output.files);
    assert!(output
        .async_facades
        .values()
        .all(|p| p == &output.entries["entry"]));
    assert!(!output.modules.contains_key(Path::new("child.mjs")));
    observe_activated(
        output,
        "assert.deepEqual(trace, []); assert.equal(await entry.load(false), false); \
         assert.deepEqual(trace, []); const pending = entry.load(true); \
         assert.deepEqual(trace, []); const a = await pending; const b = await entry.load(true); \
         assert.deepEqual(a.slice(0,2), [1,2]); assert.deepEqual(b.slice(0,2), [2,3]); \
         assert.equal(a[2], b[2]); assert.equal(a[3], b[3]); \
         assert.equal(await entry.other(), 3); assert.deepEqual(trace, ['child']);",
    );
}

#[test]
fn single_parent_public_esm_static_island_order_and_cached_undefined_failure() {
    for failure in [false, true] {
        let input = sources(&[
            ("entry.mjs",
             "export async function load() { try { return (await import('./child.mjs')).value; } \
              catch (error) { return {error}; } }"),
            ("child.mjs",
             "import './z.mjs'; import './a.mjs'; export {value} from './a.mjs'; trace.push('child');"),
            ("z.mjs", "import './shared.mjs'; trace.push('z');"),
            ("a.mjs", "import './shared.mjs'; trace.push('a'); export const value = 3;"),
            ("shared.mjs", if failure { "trace.push('shared'); throw undefined;" } else { "trace.push('shared');" }),
        ]);
        let output = automatically_hoisted(&input);
        assert_eq!(output.files.len(), 1);
        observe_activated(
            output,
            if failure {
                "assert.deepEqual(trace, []); const [a,b] = await Promise.all([entry.load(),entry.load()]); \
             assert.deepEqual(a, {error:undefined}); assert.deepEqual(b,a); \
             assert.deepEqual(await entry.load(),a); assert.deepEqual(trace,['shared']);"
            } else {
                "assert.deepEqual(trace, []); assert.equal(await entry.load(),3); \
             assert.equal(await entry.load(),3); assert.deepEqual(trace,['shared','z','a','child']);"
            },
        );
    }
}

#[test]
fn single_parent_public_esm_empty_and_destructured_targets() {
    for (child, load, expected) in [
        ("", "await import('./child.mjs'); return 1;", 1),
        (
            "export const value = 2;",
            "const {value} = await import('./child.mjs'); return value;",
            2,
        ),
    ] {
        let input = sources(&[
            (
                "entry.mjs",
                &format!("export async function load() {{ {load} }}"),
            ),
            ("child.mjs", child),
        ]);
        let output = automatically_hoisted(&input);
        assert_eq!(output.files.len(), 1);
        observe_activated(
            output,
            &format!("assert.equal(await entry.load(),{expected});"),
        );
    }
}

#[test]
fn single_parent_public_esm_multiple_parents_and_eager_targets_stay_native() {
    for entry in [
        "export {other} from './other.mjs'; export async function load() { return (await import('./child.mjs')).value; }",
        "import './child.mjs'; export async function load() { return (await import('./child.mjs')).value; }",
        "export {value} from './child.mjs'; export async function load() { return (await import('./child.mjs')).value; }",
    ] {
        let input = sources(&[
            ("entry.mjs", entry),
            ("other.mjs", "export async function other() { return (await import('./child.mjs')).value; }"),
            ("child.mjs", "trace.push('child'); export const value = 1;"),
        ]);
        let output = automatically_hoisted(&input);
        assert!(!output.files.values().any(|s| s.contains("Unsupported deferred ESM reentry")));
        observe_activated(output, "assert.equal(await entry.load(),1); assert.deepEqual(trace,['child']);");
    }
    let input = sources(&[
        (
            "entry.mjs",
            "export async function load() { return (await import('./child.mjs')).value; }",
        ),
        ("child.mjs", "export const value = 1;"),
    ]);
    let (_, output) = build(&input, &[("entry.mjs", true), ("child.mjs", true)], false);
    assert!(!output
        .files
        .values()
        .any(|s| s.contains("Unsupported deferred ESM reentry")));
}

#[test]
fn single_parent_public_esm_unsupported_observations_and_islands_stay_native() {
    for usage in [
        "return import('./child.mjs');",
        "const ns = await import('./child.mjs'); return ns;",
        "const ns = await import('./child.mjs'); return Object.keys(ns);",
        "const ns = await import('./child.mjs'); return Object.getOwnPropertyDescriptor(ns,'value');",
        "const ns = await import('./child.mjs'); return ns[globalThis.key];",
        "const ns = await import('./child.mjs'); ns.value = 2;",
        "const ns = await import('./child.mjs'); delete ns.value;",
        "const {...ns} = await import('./child.mjs'); return ns;",
        "return (await import('./child.mjs')).missing;",
    ] {
        let input = sources(&[
            ("entry.mjs", &format!("export async function load() {{ {usage} }}")),
            ("child.mjs", "export const value = 1;"),
        ]);
        let output = automatically_hoisted(&input);
        assert!(!output.files.values().any(|s| s.contains("Unsupported deferred ESM reentry")), "{usage}");
        assert!(output.modules.contains_key(Path::new("child.mjs")));
    }
    for child in [
        "export const value = await Promise.resolve(1);",
        "export const value = 1; export function then(resolve) { resolve(1); }",
        "import './dep.mjs'; export const value = 1;",
        "export function value() { return this; }",
        "export const value = globalThis.fn;",
        "export function value() {} value = globalThis.fn;",
    ] {
        let input = sources(&[
            (
                "entry.mjs",
                "export async function load() { return (await import('./child.mjs')).value(); }",
            ),
            ("child.mjs", child),
            ("dep.mjs", "import './child.mjs';"),
        ]);
        let output = automatically_hoisted(&input);
        assert!(
            !output
                .files
                .values()
                .any(|s| s.contains("Unsupported deferred ESM reentry")),
            "{child}"
        );
    }
}

#[test]
fn single_parent_native_fallback_keeps_namespace_then_tla_and_cycle_behavior() {
    for (child, usage, check) in [
        (
            "export const value = 2; export function then(resolve) { trace.push('then'); resolve({value:3}); }",
            "return (await import('./child.mjs')).value;",
            "assert.equal(await entry.load(),3); assert.deepEqual(trace,['then']);",
        ),
        (
            "trace.push('start'); export const value = await Promise.resolve(4); trace.push('done');",
            "return (await import('./child.mjs')).value;",
            "assert.equal(await entry.load(),4); assert.deepEqual(trace,['start','done']);",
        ),
        (
            "import './dep.mjs'; trace.push('child'); export const value = 5;",
            "return (await import('./child.mjs')).value;",
            "assert.equal(await entry.load(),5); assert.deepEqual(trace,['dep','child']);",
        ),
        (
            "export let value = 6; export function change() { value++; }",
            "const ns = await import('./child.mjs'); return ns;",
            "const ns = await entry.load(); assert.equal(ns,await entry.load()); \
             assert.equal(Object.getPrototypeOf(ns),null); assert.equal(Object.isExtensible(ns),false); \
             assert.deepEqual(Object.getOwnPropertyDescriptor(ns,'value'), \
             {value:6,writable:true,enumerable:true,configurable:false}); \
             ns.change(); assert.equal(ns.value,7); assert.throws(()=>{ns.value=8},TypeError);",
        ),
    ] {
        let input = sources(&[
            ("entry.mjs", &format!("export async function load() {{ {usage} }}")),
            ("child.mjs", child),
            ("dep.mjs", "import './child.mjs'; trace.push('dep');"),
        ]);
        let output = automatically_hoisted(&input);
        assert!(!output.files.values().any(|s| s.contains("Unsupported deferred ESM reentry")));
        observe_activated(output, check);
    }
}

#[test]
fn single_parent_generated_names_avoid_source_exports_and_named_collisions_fall_back() {
    let helper = format!("__gf_init_{}", digest("child.mjs"));
    let input = sources(&[
        (
            "entry.mjs",
            &format!(
                "export const {helper} = 7; \
         export async function load() {{ return (await import('./child.mjs')).value; }}"
            ),
        ),
        ("child.mjs", "export const value = (trace.push('child'),8);"),
    ]);
    let output = automatically_hoisted(&input);
    assert_eq!(output.files.len(), 1);
    observe_activated(
        output,
        &format!("assert.equal(entry.{helper},7); assert.equal(await entry.load(),8);"),
    );
    let input = sources(&[
        (
            "entry.mjs",
            "export function read() {} \
         export async function load() { return (await import('./child.mjs')).read(); }",
        ),
        ("child.mjs", "export function read() { return read.name; }"),
    ]);
    let output = automatically_hoisted(&input);
    assert!(!output
        .files
        .values()
        .any(|s| s.contains("Unsupported deferred ESM reentry")));
    observe_activated(output, "assert.equal(await entry.load(),'read');");
}

#[test]
fn activated_esm_empty_local_target_initializes_without_retained_segments() {
    let input = sources(&[
        (
            "entry.mjs",
            "export function load() { return import('./child.mjs'); }",
        ),
        ("child.mjs", ""),
    ]);
    let output = activated(&input, &[("child.mjs", "entry.mjs")])
        .render()
        .unwrap();
    assert_eq!(output.files.len(), 1);
    observe_activated(
        output,
        "const [a, b] = await Promise.all([entry.load(), entry.load()]);\n\
         assert.equal(a, b);\n\
         assert.deepEqual(Object.keys(a), []);\n\
         assert.equal(await entry.load(), a);",
    );
}

#[test]
fn activated_esm_local_import_is_async_guarded_once_and_live() {
    let input = sources(&[
        (
            "entry.mjs",
            "export const Promise = null; \
             export function load(enabled = true) { return enabled && import('./child.mjs'); }",
        ),
        (
            "child.mjs",
            "const secret = (trace.push('child'), {}); export let count = 0; \
         export function read() { return secret; } export function increment() { return ++count; } \
         export function receiver() { return this; }",
        ),
    ]);
    let output = activated(&input, &[("child.mjs", "entry.mjs")])
        .render()
        .unwrap();
    assert_eq!(output.files.len(), 1);
    observe_activated(
        output,
        "assert.deepEqual(trace, []);\n\
         assert.equal(entry.load(false), false);\n\
         assert.deepEqual(trace, []);\n\
         const first = entry.load(), second = entry.load();\n\
         assert.deepEqual(trace, []);\n\
         const [a, b] = await Promise.all([first, second]);\n\
         assert.equal(a, b);\n\
         assert.equal(a.receiver(), a);\n\
         const read = a.read, secret = read();\n\
         assert.equal(typeof secret, 'object');\n\
         assert.equal(a.increment(), 1);\n\
         assert.equal(a.count, 1);\n\
         assert.equal(await entry.load(), a);\n\
         assert.equal(a.read, read); assert.equal(a.read(), secret);\n\
         assert.deepEqual(trace, ['child']);",
    );
}

#[test]
fn activated_esm_recipes_preserve_guards_receivers_and_argument_order() {
    let input = sources(&[
        (
            "entry.mjs",
            "export async function invoke(enabled) { \
         return enabled && sink(first(), (await import('./child.mjs')).method(second())); } \
         export function unused(enabled) { return enabled && import('./unused.mjs'); }",
        ),
        (
            "child.mjs",
            "trace.push('child'); export const value = 3; \
         export function method(arg) { trace.push('method'); return this.value + arg; }",
        ),
        (
            "unused.mjs",
            "trace.push('unused'); export const unused = 1;",
        ),
    ]);
    for owner in ["entry.mjs", "child.mjs"] {
        let output = activated(&input, &[("child.mjs", owner), ("unused.mjs", owner)])
            .render()
            .unwrap();
        observe_activated(
            output,
            "globalThis.first = () => (trace.push('first'), 1);\n\
             globalThis.second = () => (trace.push('second'), 2);\n\
             globalThis.sink = (a, b) => (trace.push('sink'), a + b);\n\
             assert.equal(await entry.invoke(false), false);\n\
             assert.equal(entry.unused(false), false);\n\
             assert.deepEqual(trace, []);\n\
             assert.equal(await entry.invoke(true), 6);\n\
             assert.deepEqual(trace, ['first', 'child', 'second', 'method', 'sink']);",
        );
    }
}

#[test]
fn activated_esm_cross_chunk_requests_survive_final_naming() {
    let input = sources(&[
        (
            "entry.mjs",
            "export function load() { return import('./child.mjs'); }",
        ),
        ("child.mjs", "trace.push('child'); export const value = {};"),
    ]);
    let planned = activated(&input, &[("child.mjs", "child.mjs")]);
    let destinations = planned
        .files
        .keys()
        .enumerate()
        .map(|(index, path)| {
            (
                path.clone(),
                PathBuf::from(format!("renamed/depth-{index}/output.mjs")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut output = planned.render_to(&destinations).unwrap();
    for path in output.entries.values_mut() {
        *path = destinations[path].clone();
    }
    let target = destinations[&output.async_facades.values().next().unwrap().clone()].clone();
    observe_activated(
        output,
        &format!(
            "assert.deepEqual(trace, []);\n\
         await import({});\n\
         assert.deepEqual(trace, []);\n\
         const pending = entry.load();\n\
         assert.deepEqual(trace, []);\n\
         const [a, b] = await Promise.all([pending, entry.load()]);\n\
         assert.equal(a, b); assert.equal(typeof a.value, 'object');\n\
         assert.equal(await entry.load(), a);\n\
         assert.deepEqual(trace, ['child']);",
            quote(&format!("./{}", target.display())),
        ),
    );
}

#[test]
fn activated_esm_missing_native_chunk_rejects_at_the_original_use() {
    let input = sources(&[
        (
            "entry.mjs",
            "export async function load() { \
         try { return await import('./child.mjs'); } catch (error) { \
         trace.push('caught'); return error.code; } }",
        ),
        ("child.mjs", "trace.push('child'); export const value = 1;"),
    ]);
    let mut output = activated(&input, &[("child.mjs", "child.mjs")])
        .render()
        .unwrap();
    let target = output.async_facades.values().next().unwrap().clone();
    output.files.remove(&target).unwrap();
    observe_activated(
        output,
        "assert.deepEqual(trace, []);\n\
         assert.equal(await entry.load(), 'ERR_MODULE_NOT_FOUND');\n\
         assert.deepEqual(trace, ['caught']);",
    );
}

#[test]
fn activated_esm_static_dependencies_activate_inside_the_dependent_in_source_order() {
    let input = sources(&[
        (
            "entry.mjs",
            "export function load() { return import('./child.mjs'); }",
        ),
        (
            "child.mjs",
            "import * as zns from './z.mjs'; import {a} from './a.mjs'; \
         trace.push('child'); export const value = zns.z + a;",
        ),
        (
            "z.mjs",
            "import './shared.mjs'; trace.push('z'); export const z = 1;",
        ),
        (
            "a.mjs",
            "import './shared.mjs'; trace.push('a'); export const a = 2;",
        ),
        ("shared.mjs", "trace.push('shared');"),
    ]);
    for owner in ["entry.mjs", "child.mjs"] {
        for colocated_dependencies in [false, true] {
            let mut placements = vec![("child.mjs", owner)];
            if colocated_dependencies {
                placements.extend([("z.mjs", owner), ("a.mjs", owner), ("shared.mjs", owner)]);
            }
            let output = activated(&input, &placements).render().unwrap();
            observe_activated(
                output,
                "assert.deepEqual(trace, []);\n\
             const [a, b] = await Promise.all([entry.load(), entry.load()]);\n\
             assert.equal(a, b); assert.equal(a.value, 3);\n\
             assert.deepEqual(trace, ['shared', 'z', 'a', 'child']);",
            );
        }
    }
}

#[test]
fn activated_esm_static_failure_and_reexport_reads_share_the_dependency_guard() {
    let input = sources(&[
        (
            "entry.mjs",
            "export function load() { return import('./child.mjs'); }",
        ),
        (
            "child.mjs",
            "import './dep.mjs'; import {value} from './dep.mjs'; \
             export {value, change} from './dep.mjs'; trace.push('child');",
        ),
        (
            "dep.mjs",
            "trace.push('dep'); export let value = 1; export function change() { value++; }",
        ),
    ]);
    let output = activated(&input, &[("child.mjs", "entry.mjs")])
        .render()
        .unwrap();
    observe_activated(
        output,
        "assert.deepEqual(trace, []);\n\
             const a = await entry.load(); assert.equal(a.value, 1);\n\
             a.change(); assert.equal(a.value, 2);\n\
             assert.deepEqual(trace, ['dep', 'child']);\n\
             assert.equal(await entry.load(), a);",
    );

    let mut input = input;
    input.insert(
        PathBuf::from("dep.mjs"),
        "trace.push('dep'); throw undefined; export let value = 1; export function change() {}"
            .into(),
    );
    let output = activated(&input, &[("child.mjs", "entry.mjs")])
        .render()
        .unwrap();
    observe_activated(
        output,
        "const results = await Promise.allSettled([entry.load(), entry.load(), entry.load()]);\n\
             for (const result of results) {\n\
               assert.equal(result.status, 'rejected'); assert.equal(result.reason, undefined);\n\
             }\n\
             assert.deepEqual(trace, ['dep']);",
    );
}

#[test]
fn activated_esm_failure_rejects_in_original_try_context_and_is_cached() {
    for owner in ["entry.mjs", "child.mjs"] {
        for thrown in ["undefined", "null", "0", "'failure'", "{reason: 1}"] {
            let input = sources(&[
                (
                    "entry.mjs",
                    "export function load() { return import('./child.mjs'); } \
                 export async function caught() { try { await import('./child.mjs'); } \
                 catch (error) { trace.push('caught'); return {error}; } }",
                ),
                (
                    "child.mjs",
                    "trace.push('child'); throw globalThis.thrown; export const value = 1;",
                ),
            ]);
            let output = activated(&input, &[("child.mjs", owner)]).render().unwrap();
            observe_activated(
                output,
                &format!(
                    "globalThis.thrown = {thrown};\n\
                 const first = entry.load(), second = entry.load();\n\
                 assert.deepEqual(trace, []);\n\
                 const results = await Promise.allSettled([first, second]);\n\
                 for (const result of results) {{\n\
                   assert.equal(result.status, 'rejected'); assert.equal(result.reason, thrown);\n\
                 }}\n\
                 assert.deepEqual(await entry.caught(), {{error: thrown}});\n\
                 assert.deepEqual(await entry.caught(), {{error: thrown}});\n\
                 assert.deepEqual(trace, ['child', 'caught', 'caught']);"
                ),
            );
        }
    }
}

#[test]
fn activated_esm_unsupported_targets_keep_native_boundaries() {
    for source in [
        "export const value = await Promise.resolve(1);",
        "export class Thing {}",
        "export const {x} = {x: 1};",
        "export default function () {}",
        "export const value = 1; export function write() { value = 2; }",
        "export function then(resolve) { resolve(1); }",
        "export * as namespace from './dep.mjs';",
        "import * as ns from './dep.mjs'; export const value = ns;",
        "import './dep.mjs'; export const value = 1;",
    ] {
        let input = sources(&[
            (
                "entry.mjs",
                "export function load() { return import('./child.mjs'); }",
            ),
            ("child.mjs", source),
            ("dep.mjs", "import './child.mjs'; export const value = 1;"),
        ]);
        let output = activated(&input, &[("child.mjs", "entry.mjs")])
            .render()
            .unwrap();
        assert!(
            !output
                .files
                .values()
                .any(|code| code.contains("Unsupported deferred ESM reentry")),
            "{source}"
        );
    }
    let input = sources(&[
        (
            "entry.mjs",
            "export function read() { return import('./child.mjs'); }",
        ),
        ("child.mjs", "export function read() { return 1; }"),
    ]);
    let output = activated(&input, &[("child.mjs", "entry.mjs")])
        .render()
        .unwrap();
    assert!(!output
        .files
        .values()
        .any(|code| code.contains("Unsupported deferred ESM reentry")));
}

#[test]
fn native_concatenation_preserves_late_import_statement_boundaries() {
    assert_native_observation(
        &sources(&[
            ("dep.mjs", "console.log('dep');"),
            (
                "entry.mjs",
                "const value = 1\nimport './dep.mjs';\n[0].toString();\nconsole.log(value);",
            ),
        ]),
        "dep\n1\n",
    );
}

#[test]
fn native_concatenation_preserves_late_export_statement_boundaries() {
    for separator in [
        "export {}",
        "export {value}",
        "export {item} from './dep.mjs'",
        "export * from './dep.mjs'",
    ] {
        assert_native_observation(
            &sources(&[
                ("dep.mjs", "console.log('dep'); export const item = 2;"),
                ("entry.mjs", &format!("import './dep.mjs';\nconst value = 1\n{separator};\n[0].toString();\nconsole.log(value);")),
            ]),
            "dep\n1\n",
        );
    }
}

#[test]
fn native_concatenation_preserves_boundaries_after_dead_and_pure_removal() {
    for removed in ["const dead = 2;", "/*#__PURE__*/ Math.abs(2);"] {
        assert_native_observation(
            &sources(&[
                ("entry.mjs", "import './dep.mjs';"),
                (
                    "dep.mjs",
                    &format!("const value = 1\n{removed}\n[0].toString();\nconsole.log(value);"),
                ),
            ]),
            "1\n",
        );
    }
}

#[test]
fn native_concatenation_separates_module_ends_outside_comments_and_nested_syntax() {
    for ending in ["", " // trailing comment", "\n//# sourceURL=obsolete.js"] {
        assert_native_observation(
            &sources(&[
                ("dep.mjs", &format!("console.log('dep'){ending}")),
                ("entry.mjs", "#!/usr/bin/env node\n'use strict';\nimport './dep.mjs';\n[0].toString();\nconsole.log(/* internal comment */ (1 +\n2));\nif (true) console.log('then')\nelse console.log('else');\nconsole.log('entry') // final comment"),
            ]),
            "dep\n3\nthen\nentry\n",
        );
    }
}

#[test]
fn lexical_rename_preserves_proto_literal_binding_and_assignment_shorthand() {
    let source = "let {__proto__ = null} = {['__proto__']: null}; console.log(Object.keys({__proto__}), Object.getPrototypeOf({__proto__}) === Object.prototype); ({__proto__ = 4} = {}); console.log(__proto__ === Object.prototype);";
    let path = Path::new("entry.mjs");
    let parsed =
        parse_module(&logger::StdioLogger::new(), source, path, ModuleFormat::Esm).unwrap();
    let id = parsed
        .facts
        .bindings
        .iter()
        .find(|b| b.name == "__proto__")
        .unwrap()
        .id;
    let renamed = crate::rewrite::rename_private(path, source, &[(id, "renamed".into())]).unwrap();
    let baseline = node(&sources(&[("entry.mjs", source)]), path);
    let observed = node(&sources(&[("entry.mjs", &renamed)]), path);
    for result in [&baseline, &observed] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(observed.stdout, baseline.stdout);
    assert_eq!(observed.stdout, b"[ '__proto__' ] true\ntrue\n");
}

fn assert_native_observation(sources: &BTreeMap<PathBuf, String>, expected: &str) {
    let (_, output) = build(sources, &[("entry.mjs", true)], false);
    assert_eq!(output.backend, super::Backend::Native);
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.entries["entry.mjs"], Path::new("native.mjs"));
    let baseline = node(sources, Path::new("entry.mjs"));
    let observed = node(&output.files, &output.entries["entry.mjs"]);
    for result in [&baseline, &observed] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(observed.stdout, baseline.stdout);
    assert_eq!(observed.stdout, expected.as_bytes());
}

#[test]
fn native_chunks_materialize_split_exports_and_preserve_order_and_live_bindings() {
    let sources = sources(&[
        ("dep.mjs", "export let a = 1, b = a + 1; const dead = 9, used = 3; export function bump(){ a++; b += used; }"),
        ("entry.mjs", "import {a, b, bump} from './dep.mjs'; const values = [], first = values.push(a), second = values.push(b); bump(); console.log(values.join(','), a, b, first, second); export {a, b};"),
    ]);
    assert_native_observation(&sources, "1,2 2 5 1 2\n");
    let (_, output) = build(&sources, &[("entry.mjs", true), ("dep.mjs", true)], false);
    let observed = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, b"1,2 2 5 1 2\n");
    assert!(output.files.values().all(|text| !text.contains("dead")));
}

#[test]
fn native_concatenation_uses_canonical_live_bindings_and_exact_public_names() {
    let sources = sources(&[
        ("leaf.mjs", "console.log(hoisted()); var value = 1; var value = 2; export {value as default}; export function increment() { function identity(value) { return value; } value += identity(1); } export function hoisted() { return later; } var later = 7; var later; console.log(typeof key);"),
        ("barrel.mjs", "export {default as original, increment as bump, hoisted as early} from './leaf.mjs';"),
        ("entry.mjs", "import {original as current, bump as step, early as read} from './barrel.mjs'; const value = 9; const {key = 4} = {}; const object = {value, key}; export {current as public, step as mutate, read as early}; export const result = JSON.stringify(object);"),
    ]);
    let (plan, output) = build(&sources, &[("entry.mjs", true)], false);
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.entries["entry.mjs"], Path::new("native.mjs"));
    assert_eq!(
        output.modules.len(),
        1,
        "private sources have no namespace facade"
    );
    let body = &output.files[Path::new("native.mjs")];
    assert!(body.contains(" = 1;"));
    assert!(body.contains(" = 2;"));
    assert!(body.contains("function increment()"));
    assert!(!body.contains("from "));
    assert!(!body.contains(".alias("));
    assert!(output
        .members
        .values()
        .all(|p| p == Path::new("native.mjs")));
    assert_eq!(
        output.members.len(),
        plan.chunks.values().map(Vec::len).sum::<usize>()
    );
    let check = |entry: &Path| {
        format!(
        "import * as ns from './{}'; console.log(Object.keys(ns).join(','), ns.public, ns.early(), ns.result); ns.mutate(); console.log(ns.public);",
        entry.display()
    )
    };
    let mut native = sources.clone();
    native.insert("check.mjs".into(), check(Path::new("entry.mjs")));
    let baseline = node(&native, Path::new("check.mjs"));
    let mut bundled = output.files.clone();
    bundled.insert("check.mjs".into(), check(&output.entries["entry.mjs"]));
    let observed = node(&bundled, Path::new("check.mjs"));
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
        b"undefined\nundefined\nearly,mutate,public,result 2 7 {\"value\":9,\"key\":4}\n3\n"
    );
    assert_eq!(output, build(&sources, &[("entry.mjs", true)], true).1);
}

#[test]
fn native_concatenation_keeps_static_source_order_names_and_free_globals() {
    let sources = sources(&[
        ("z.mjs", "console.log('first'); export default function Named() {} export class Model {}"),
        ("pure.mjs", "export {default as call, Model as Type} from './z.mjs';"),
        ("a.mjs", "console.log('second'); export const globalName = 8;"),
        ("entry.mjs", "#!/usr/bin/env node\n'use strict'; import {call as invoke, Type as Constructor} from './pure.mjs'; import {globalName as local} from './a.mjs'; console.log(invoke.name, Constructor.name, typeof globalName, local, this === undefined); export {invoke as default};"),
    ]);
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    assert_eq!(output.files.len(), 1);
    let body = &output.files[Path::new("native.mjs")];
    assert!(body.starts_with("#!/usr/bin/env node\n"));
    assert!(!body.contains("export {default as call"));
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
        b"first\nsecond\nNamed Model undefined 8 true\n"
    );
}

#[test]
fn native_boundaries_preserve_observable_names_and_readonly_import_writes() {
    for (dependency, entry, expected) in [
        ("export function Same() {} export class Model {}", "import {Same as other, Model as Other} from './dep.mjs'; function Same() {} class Model {} console.log(Same.name, other.name, Model.name, Other.name);", "Same Same Model Model\n"),
        ("export const fn = 1;", "import {fn as value} from './dep.mjs'; const fn = () => value; console.log(fn.name);", "fn\n"),
        ("export let value = 1;", "import {value as local} from './dep.mjs'; try {local = 2;} catch(e) {console.log(e.name);} console.log(local);", "TypeError\n1\n"),
    ] {
        let sources = sources(&[("dep.mjs", dependency), ("entry.mjs", entry)]);
        let before = native::global_preparation_count();
        let (_, output) = build(&sources, &[("entry.mjs", true)], false);
        assert_eq!(native::global_preparation_count() - before, 1);
        assert_eq!(output.backend, super::Backend::Native);
        let observed = node(&output.files, &output.entries["entry.mjs"]);
        assert!(observed.status.success(), "{}", String::from_utf8_lossy(&observed.stderr));
        assert_eq!(observed.stdout, expected.as_bytes());
    }
}

#[test]
fn native_shared_defaults_avoid_global_names_across_owners_and_input_orders() {
    let source = sources(&[
        ("a.mjs", "export let value = 1; export default value; export function bump() { value++; }"),
        ("z.mjs", "export default (() => 2);"),
        ("blockers.mjs", "export const __gf_default_0 = 10; export {__gf_default_0 as __gf_default_1}; export function read(__gf_default_2) { return [__gf_default_2, typeof __gf_default_3, typeof __gf_import_0]; }"),
        ("entry.mjs", "import snapshot, {value, bump} from './a.mjs'; import fn from './z.mjs'; import {__gf_default_1 as blocker, read} from './blockers.mjs'; bump(); console.log(snapshot, value, fn.name, fn(), blocker, read(5).join(',')); export {snapshot, value, fn};"),
    ]);
    let baseline = node(&source, Path::new("entry.mjs"));
    assert!(baseline.status.success());
    assert_eq!(baseline.stdout, b"1 2 default 2 10 5,undefined,undefined\n");
    for entries in [
        vec![("entry.mjs", true)],
        vec![("entry.mjs", true), ("blockers.mjs", true)],
    ] {
        let mut previous = None;
        for reverse in [false, true] {
            let before = native::global_preparation_count();
            let (_, output) = build(&source, &entries, reverse);
            assert_eq!(native::global_preparation_count() - before, 1);
            assert_eq!(output.files.len() == 1, entries.len() == 1);
            let names = if entries.len() == 1 {
                &["__gf_default_4", "__gf_default_5"][..]
            } else {
                // a.mjs retains native default syntax, but its allocation still
                // precedes the relocated z.mjs expression.
                &["__gf_default_5"][..]
            };
            for name in names {
                assert!(
                    output.files.values().any(|text| text.contains(name)),
                    "missing allocated default {name}: {:?}",
                    output.files
                );
            }
            let actual = node(&output.files, &output.entries["entry.mjs"]);
            assert!(
                actual.status.success(),
                "{}",
                String::from_utf8_lossy(&actual.stderr)
            );
            assert_eq!(actual.stdout, baseline.stdout);
            if let Some(previous) = previous {
                assert_eq!(output, previous);
            }
            previous = Some(output);
        }
    }
}

#[test]
fn native_shared_import_exclusions_preserve_free_names_and_live_bindings() {
    let alias = format!("__gf_segment_import_{}_8", digest("entry.mjs"));
    let source = sources(&[
        ("state.mjs", &format!("console.log('state'); export let value = 1; export function bump() {{ value++; }} export {{value as {alias}__}};")),
        ("blocker.mjs", &format!("console.log('blocker'); export function read({alias}_) {{ return {alias}_; }}")),
        ("entry.mjs", &format!("import {{value, bump}} from './state.mjs'; import {{read}} from './blocker.mjs'; bump(); console.log(value, typeof {alias}, read(5));")),
    ]);
    let baseline = node(&source, Path::new("entry.mjs"));
    assert!(baseline.status.success());
    assert_eq!(baseline.stdout, b"state\nblocker\n2 undefined 5\n");
    let entries = [("entry.mjs", true), ("state.mjs", true)];
    let mut previous = None;
    for reverse in [false, true] {
        let before = native::global_preparation_count();
        let (_, output) = build(&source, &entries, reverse);
        assert_eq!(native::global_preparation_count() - before, 1);
        let actual = node(&output.files, &output.entries["entry.mjs"]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
        if let Some(previous) = previous {
            assert_eq!(output, previous);
        }
        previous = Some(output);
    }
}

#[test]
fn native_defaults_snapshot_expressions_but_keep_binding_aliases_live() {
    let sources = sources(&[
            ("snapshot.mjs", "export let value = 1; export default value; export function bump() { value++; }"),
            ("live.mjs", "export let current = 3; export {current as default}; export function step() { current++; }"),
            ("barrel.mjs", "export {default as snapshot, value, bump} from './snapshot.mjs'; export {default as live, step} from './live.mjs';"),
            ("entry.mjs", "import {snapshot, value, bump, live, step} from './barrel.mjs'; bump(); step(); console.log(snapshot, value, live); export {snapshot as 'snapshot-key', live as default};"),
        ]);
    assert_native_observation(&sources, "1 2 4\n");
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    assert_eq!(output, build(&sources, &[("entry.mjs", true)], true).1);
    let mut files = output.files;
    files.insert("check.mjs".into(), "import * as ns from './native.mjs'; console.log(Object.keys(ns).join(','), ns['snapshot-key'], ns.default, Object.getPrototypeOf(ns) === null, Object.isExtensible(ns));".into());
    let observed = node(&files, Path::new("check.mjs"));
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(
        observed.stdout,
        b"1 2 4\ndefault,snapshot-key 1 4 true false\n"
    );
}

#[test]
fn native_default_value_forms_keep_native_observable_names() {
    for (definition, expected) in [
        ("export default function() {}", "default"),
        ("export default async function() {}", "default"),
        ("export default function*() {}", "default"),
        (
            "export default class { static seen = this.name; }",
            "default",
        ),
        ("export default (() => 1);", "default"),
        ("export default (function() {});", "default"),
        ("export default (0, function() {});", ""),
        ("export default (true ? () => 1 : () => 2);", ""),
        (
            "export default class Named { static seen = this.name; }",
            "Named",
        ),
        ("export default function Named() {}", "Named"),
    ] {
        assert_native_observation(
            &sources(&[
                ("dep.mjs", definition),
                ("entry.mjs", "import value from './dep.mjs'; const fn = () => value; const object = {fn}; if (value.seen !== undefined && value.seen !== value.name) throw Error('class static name'); const descriptor = Object.getOwnPropertyDescriptor(value, 'name'); console.log(value.name, object.fn.name, descriptor.writable, descriptor.enumerable, descriptor.configurable);"),
            ]),
            &format!("{expected} fn false false true\n"),
        );
    }
    assert_native_observation(
        &sources(&[("entry.mjs", "try { Named; } catch (e) { console.log(e.name); } export default class Named { static seen = this.name; } console.log(Named.name, Named.seen);")]),
        "ReferenceError\nNamed Named\n",
    );
    assert_native_observation(
        &sources(&[("entry.mjs", "const __gf_default_0 = 8; export default class { static seen = this.name; static value = __gf_default_0; } console.log(__gf_default_0);")]),
        "8\n",
    );
    assert_native_observation(
        &sources(&[
            ("dep.mjs", "export default function Named() {} Named = 42;"),
            (
                "entry.mjs",
                "import value from './dep.mjs'; console.log(value);",
            ),
        ]),
        "42\n",
    );
}

#[test]
fn native_default_expressions_keep_evaluation_time_and_throwing_tdz() {
    for (declaration, expected) in [
        ("export default before; const before = 1;", "before"),
        (
            "export default class { static value = before; } const before = 1;",
            "before",
        ),
    ] {
        let sources = sources(&[("entry.mjs", declaration)]);
        let (_, output) = build(&sources, &[("entry.mjs", true)], false);
        assert_eq!(output.entries["entry.mjs"], Path::new("native.mjs"));
        for result in [
            node(&sources, Path::new("entry.mjs")),
            node(&output.files, &output.entries["entry.mjs"]),
        ] {
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr)
                .contains(&format!("Cannot access '{expected}' before initialization")));
        }
    }
    assert_native_observation(
        &sources(&[
            ("dep.mjs", "console.log('before'); export default (console.log('expression'), 3); console.log('after');"),
            ("entry.mjs", "import value from './dep.mjs'; console.log(value);"),
        ]),
        "before\nexpression\nafter\n3\n",
    );
}

#[test]
fn native_namespace_property_reads_follow_aliases_and_live_bindings() {
    let sources = sources(&[
            ("dep.mjs", "export let value = 1; export function bump() { value++; } export default () => 2;"),
            ("barrel.mjs", "export * as nested from './dep.mjs';"),
            ("entry.mjs", "import * as ns from './dep.mjs'; import {nested as alias} from './barrel.mjs'; const bump = ns.bump; const fn = ns.default; bump(); console.log(ns.value, alias.value, fn.name, fn(), ns.bump === alias.bump); export {fn as 'public-name'};"),
        ]);
    assert_native_observation(&sources, "2 2 default 2 true\n");
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    let mut files = output.files;
    files.insert(
        "check.mjs".into(),
        "import * as ns from './native.mjs'; console.log(Object.keys(ns).join(','));".into(),
    );
    let observed = node(&files, Path::new("check.mjs"));
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, b"2 2 default 2 true\npublic-name\n");
}

#[test]
fn native_boundaries_keep_namespace_escapes_receivers_and_writes_native() {
    for (entry, expected) in [
        ("console.log(ns === alias, Object.getPrototypeOf(ns) === null, Object.isExtensible(ns)); const d = Object.getOwnPropertyDescriptor(ns, 'value'); console.log(d.value, d.writable, d.enumerable, d.configurable, Object.prototype.toString.call(ns)); export {ns as public};", "true true false\n1 true true false [object Module]\n"),
        ("console.log(ns.read());", "1\n"),
        ("try { ns.value = 2; } catch (e) { console.log(e.name); } console.log(ns.value);", "TypeError\n1\n"),
        ("try { ns.value++; } catch (e) { console.log(e.name); } console.log(ns.value);", "TypeError\n1\n"),
        ("try { delete ns.value; } catch (e) { console.log(e.name); } console.log(ns.value);", "TypeError\n1\n"),
        ("try { ({value: ns.value} = {value: 2}); } catch (e) { console.log(e.name); } console.log(ns.value);", "TypeError\n1\n"),
        ("console.log(ns.read`tag`);", "1\n"),
        ("console.log(ns.missing);", "undefined\n"),
    ] {
        let sources = sources(&[
            ("dep.mjs", "export let value = 1; export function read() { return this.value; }"),
            ("barrel.mjs", "export * as nested from './dep.mjs';"),
            ("entry.mjs", &format!("import * as ns from './dep.mjs'; import {{nested as alias}} from './barrel.mjs'; {entry}")),
        ]);
        let (_, output) = build(&sources, &[("entry.mjs", true)], false);
        assert_eq!(output.backend, super::Backend::Native);
        let baseline = node(&sources, Path::new("entry.mjs"));
        let observed = node(&output.files, &output.entries["entry.mjs"]);
        for result in [&baseline, &observed] {
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        }
        assert_eq!(observed.stdout, baseline.stdout);
        assert_eq!(observed.stdout, expected.as_bytes());
    }
}

#[test]
fn canonical_imports_and_public_aliases_share_definitions_not_names() {
    use source_graph::ExportIdentity;
    let sources = sources(&[
        ("leaf.mjs", "var value = 1; var value = 2; export {value as original};"),
        ("barrel.mjs", "export {original as renamed} from './leaf.mjs';"),
        ("outer.mjs", "export {renamed as twice} from './barrel.mjs';"),
        ("entry.mjs", "import {twice as local} from './outer.mjs'; import {original as alias} from './leaf.mjs'; export {local as public, alias as other};"),
    ]);
    let (graph, mut plan, symbols) = canonical(&sources, false);
    let leaf = graph.file_id(Path::new("leaf.mjs")).unwrap();
    let entry = graph.file_id(Path::new("entry.mjs")).unwrap();
    let binding = graph
        .file_facts(leaf)
        .unwrap()
        .bindings
        .iter()
        .find(|b| b.name == "value")
        .unwrap();
    let imported = &graph.file_facts(entry).unwrap().imports[0];
    let identity = ExportIdentity::Binding(leaf, binding.id);
    assert_eq!(
        symbols.imports[&(entry, imported.binding)].identity,
        identity
    );
    assert!(graph
        .file_facts(entry)
        .unwrap()
        .imports
        .iter()
        .all(|import| { symbols.imports[&(entry, import.binding)].identity == identity }));
    assert!(symbols.imports[&(entry, imported.binding)]
        .route
        .iter()
        .all(|&node| retained(&plan, node)));
    assert_eq!(symbols.exports[&entry]["public"].identity, identity);
    assert_eq!(symbols.exports[&entry]["other"].identity, identity);
    let definition = &symbols.definitions[&(leaf, binding.id)];
    assert_eq!(definition.name, "value");
    let route = &symbols.exports[&entry]["public"].route;
    assert!([SegmentKey::new(leaf, 0), SegmentKey::new(leaf, 1)]
        .iter()
        .all(|&key| route.contains(&GraphNode::Segment(key))));
    assert!(route.iter().all(|&node| retained(&plan, node)));
    assert!(route.iter().any(|node| matches!(node, GraphNode::Segment(key) if key.file_id == graph.file_id(Path::new("barrel.mjs")).unwrap())));
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    let mut files = output.files;
    files.insert(
        "check.mjs".into(),
        format!(
            "import {{public as value, other}} from './{}'; console.log(value, other);",
            output.entries["entry.mjs"].display()
        ),
    );
    let observed = node(&files, Path::new("check.mjs"));
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, b"2 2\n");
    let mut reordered = graph_with_format(&sources, true, ModuleFormat::Esm);
    assert!(reordered.remove_file(Path::new("leaf.mjs")));
    let error = crate::symbols::Symbols::link(&reordered, &plan)
        .err()
        .unwrap();
    assert!(error
        .to_string()
        .contains("stale symbol partition revision"));
    plan.retention
        .segments
        .retain(|segment| segment.key != SegmentKey::new(leaf, 1));
    let error = crate::symbols::Symbols::link(&graph, &plan).err().unwrap();
    assert!(error.to_string().contains("import route is not retained"));
}

#[test]
fn canonical_names_avoid_shadowing_globals_and_reordered_inputs() {
    let sources = sources(&[
        ("a.mjs", "export const same = 1, global = 2, shadow = 3, __gf_link_0 = 4, __gf_runtime = 5; export function named() {} export class Model {} export const arrow = () => 0;"),
        ("b.mjs", "export const same = 6; export function named(shadow) { return {same, shadow, global: typeof global}; } export class Model {}"),
        ("entry.mjs", "import * as a from './a.mjs'; import * as b from './b.mjs'; export {a,b}; console.log(a.named.name, b.named.name, a.Model.name, b.Model.name, a.arrow.name, JSON.stringify(b.named(a.same)));"),
    ]);
    let summary = |reverse| {
        let (graph, _, symbols) = canonical(&sources, reverse);
        let mut definitions: Vec<_> = symbols
            .definitions
            .iter()
            .map(|(&(file, binding), definition)| {
                let original = graph
                    .file_facts(file)
                    .unwrap()
                    .bindings
                    .iter()
                    .find(|b| b.id == binding)
                    .unwrap();
                (
                    graph.file_path(file).unwrap().to_owned(),
                    binding.0,
                    original.name.clone(),
                    definition.name.clone(),
                )
            })
            .collect();
        definitions.sort();
        let names: BTreeSet<_> = definitions.iter().map(|d| &d.3).collect();
        assert_eq!(names.len(), definitions.len());
        assert!(!names
            .iter()
            .any(|name| ["global", "shadow"].contains(&name.as_str())));
        assert!(definitions
            .iter()
            .any(|d| d.2 == "__gf_link_0" && d.3 == "__gf_link_0"));
        assert_eq!(definitions.iter().filter(|d| d.2 == "same").count(), 2);
        assert_eq!(definitions.iter().filter(|d| d.3 == "same").count(), 1);
        definitions
    };
    assert_eq!(summary(false), summary(true));
    let (_, forward) = build(&sources, &[("entry.mjs", true)], false);
    let (_, reversed) = build(&sources, &[("entry.mjs", true)], true);
    assert_eq!(forward, reversed);
    compare(&sources, "entry.mjs");
}

#[test]
fn canonical_values_preserve_namespace_default_and_cjs_boundaries() {
    use source_graph::ExportIdentity;
    let sources = sources(&[
        ("esm.mjs", "export default function () {} export const value = 1;"),
        ("expression.mjs", "export default () => 3;"),
        ("class.mjs", "export default class {}"),
        ("opaque.cjs", "exports.value = 2;"),
        ("entry.mjs", "import * as ns from './esm.mjs'; import value, {value as named} from './opaque.cjs'; import * as cjs from './opaque.cjs'; export {ns, value, named, cjs}; export {default as anonymous} from './esm.mjs'; export {default as expression} from './expression.mjs'; export {default as klass} from './class.mjs';"),
    ]);
    let (graph, _, symbols) = canonical(&sources, false);
    let entry = graph.file_id(Path::new("entry.mjs")).unwrap();
    let esm = graph.file_id(Path::new("esm.mjs")).unwrap();
    let cjs = graph.file_id(Path::new("opaque.cjs")).unwrap();
    let exports = &symbols.exports[&entry];
    assert_eq!(exports["ns"].identity, ExportIdentity::Namespace(esm));
    assert_eq!(exports["cjs"].identity, ExportIdentity::Namespace(cjs));
    assert_eq!(
        exports["value"].identity,
        ExportIdentity::CommonJs(cjs, "default".into())
    );
    assert_eq!(
        exports["named"].identity,
        ExportIdentity::CommonJs(cjs, "value".into())
    );
    assert!(
        matches!(exports["anonymous"].identity, ExportIdentity::Expression(key) if key.file_id == esm)
    );
    assert_ne!(
        exports["anonymous"].identity,
        exports["expression"].identity
    );
    assert_ne!(exports["anonymous"].identity, exports["klass"].identity);
    assert!(!symbols.definitions.keys().any(|(file, _)| *file == cjs));
    assert!(!symbols.definitions.keys().any(|(file, _)| *file == entry));
}

#[test]
fn canonical_surface_reuses_checked_star_ambiguity_and_explicit_precedence() {
    let sources = sources(&[
        (
            "a.mjs",
            "export const conflict = 1, explicit = 2, unique = 3; export default 4;",
        ),
        ("b.mjs", "export const conflict = 5, explicit = 6;"),
        (
            "entry.mjs",
            "export * from './a.mjs'; export * from './b.mjs'; export {explicit} from './a.mjs';",
        ),
    ]);
    let (graph, _, symbols) = canonical(&sources, false);
    let entry = graph.file_id(Path::new("entry.mjs")).unwrap();
    assert_eq!(
        symbols.exports[&entry]
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["explicit", "unique"]
    );
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    assert!(output.files[&output.entries["entry.mjs"]].contains("\"unique\""));
}

#[test]
fn canonical_linking_preserves_import_writes_and_default_initialization() {
    compare(
        &sources(&[
            ("leaf.mjs", "export const unused = 1;"),
            ("entry.mjs", "import {unused} from './leaf.mjs'; console.log('ready'); try { unused = 2; } catch(e) { console.log(e.name); } console.log(unused);"),
        ]),
        "entry.mjs",
    );
    compare(
        &sources(&[
            (
                "entry.mjs",
                "import './probe.mjs'; export default function () {}",
            ),
            (
                "probe.mjs",
                "import fn from './entry.mjs'; console.log(fn.name, typeof fn);",
            ),
        ]),
        "entry.mjs",
    );
    for declaration in ["() => 3", "class {}"] {
        compare(
            &sources(&[
                ("entry.mjs", &format!("import {{read}} from './probe.mjs'; export default {declaration}; read();")),
                ("probe.mjs", "import value from './entry.mjs'; try { console.log(value); } catch(e) { console.log(e.name); } export function read() { console.log(value.name); }"),
            ]),
            "entry.mjs",
        );
    }
}

fn node(files: &BTreeMap<PathBuf, String>, entry: &Path) -> Output {
    let dir = TmpDir::new();
    for (path, source) in files {
        let output = dir.root().join(path);
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(output, source).unwrap();
    }
    Command::new("node")
        .arg(dir.root().join(entry))
        .current_dir(dir.root())
        .output()
        .unwrap()
}

fn observation(output: &Artifacts, source: &str) -> String {
    let mut source = source.to_owned();
    for (name, entry) in &output.entries {
        let namespace = output
            .cjs_namespaces
            .iter()
            .find(|(module, _)| output.modules.get(*module) == Some(entry));
        let expression = match namespace {
            Some((_, path)) => format!(
                "import({}).then(ns => ns.default)",
                quote(&format!("./{}", path.display()))
            ),
            None => format!("import({})", quote(&format!("./{}", entry.display()))),
        };
        source = source.replace(&format!("import('./{name}')"), &expression);
    }
    source
}

fn compare(sources: &BTreeMap<PathBuf, String>, entry: &str) -> Artifacts {
    let (_, output) = build(sources, &[(entry, false)], false);
    let baseline = node(sources, Path::new(entry));
    let bundled = node(&output.files, &output.entries[entry]);
    assert_eq!(
        bundled.status.success(),
        baseline.status.success(),
        "{}\n{:?}",
        String::from_utf8_lossy(&bundled.stderr),
        output.files
    );
    assert_eq!(bundled.stdout, baseline.stdout);
    if !baseline.status.success() {
        assert!(
            String::from_utf8_lossy(&bundled.stderr).contains("ReferenceError"),
            "{}",
            String::from_utf8_lossy(&bundled.stderr)
        );
        if String::from_utf8_lossy(&baseline.stderr).contains("before initialization") {
            assert!(String::from_utf8_lossy(&bundled.stderr).contains("before initialization"));
        }
    }
    output
}

#[test]
fn esm_environment_probe_calls_without_read_contracts_remain_conservative() {
    for (expression, name, absent) in [
        ("module", "module", "ReferenceError"),
        ("exports", "exports", "ReferenceError"),
        ("typeof module", "module", "loaded"),
        ("typeof exports", "exports", "loaded"),
        ("typeof require", "require", "loaded"),
    ] {
        let sources = sources(&[
            (
                "entry.mjs",
                "import { helper } from './helper.mjs'; helper();",
            ),
            (
                "helper.mjs",
                &format!(
                    "function probe() {{ return {expression}; }}
                     export function helper() {{ return probe(); }}"
                ),
            ),
        ]);
        let graph = graph(&sources, false);
        assert!(
            crate::effect_analysis::analyze(&graph, &EffectPolicy::default())
                .unwrap()
                .is_empty()
        );
        let (_, output) = build(&sources, &[("entry.mjs", false)], false);
        for (host, expected) in [
            ("".to_owned(), format!("[\"{absent}\",[]]")),
            (
                format!(
                    "Object.defineProperty(globalThis, '{name}', {{ get() {{ log.push('{name}'); }} }});"
                ),
                format!("[\"loaded\",[\"{name}\"]]"),
            ),
        ] {
            let check = format!(
                "const log = []; {host}
                 const result = await import('./entry.mjs').then(() => 'loaded', error => error.name);
                 console.log(JSON.stringify([result, log]));"
            );
            let mut files = output.files.clone();
            files.insert("check.mjs".into(), observation(&output, &check));
            let actual = node(&files, Path::new("check.mjs"));
            assert!(actual.status.success(), "{:?}", actual);
            assert_eq!(String::from_utf8_lossy(&actual.stdout).trim(), expected);
        }
    }
}

#[test]
fn executable_files_expose_static_exports_without_any_source_loader() {
    let source = sources(&[
        ("src/main.js", "export {value, bump, default} from './state.js';"),
        ("src/state.js", "export var value = 1; var value; export function bump() { value++; } export default 7; const dead = 'must disappear';"),
    ]);
    let (_, mut output) = build(&source, &[("src/main.js", true)], false);
    assert!(output
        .files
        .keys()
        .all(|path| path.extension().is_some_and(|e| e == "mjs" || e == "cjs")));
    for text in output.files.values() {
        for forbidden in [
            "registerHooks",
            "experimental-loader",
            "good-fences:",
            "readFileSync",
            "JSON.parse",
            "new Function",
            "eval(",
        ] {
            assert!(!text.contains(forbidden), "{forbidden}");
        }
    }
    assert_eq!(
        output.entries["src/main.js"],
        output.modules[Path::new("src/main.js")]
    );
    let entry = quote(&format!("./{}", output.entries["src/main.js"].display()));
    output.files.insert("observe.mjs".into(), format!(
        "import value, {{value as live, bump}} from {entry}; import * as ns from {entry}; console.log(value, live, ns.value); bump(); console.log(live, ns.value, ns === await import({entry}));"
    ));
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"7 1 1\n2 2 true\n");

    compare(&sources(&[
        ("entry.mjs", "import './dep.mjs';"),
        ("dep.mjs", "const obj = { method(value) { console.log('call', value); } }; Promise.resolve().then(() => console.log('microtask')); /*#__PURE__*/ obj?.method(await Promise.resolve('argument')); console.log('done');"),
    ]), "entry.mjs");
}

#[test]
fn commonjs_native_cache_cycles_aliases_replacement_and_retry() {
    let output = compare(&sources(&[
        ("entry.cjs", "const a = require('./a.cjs'); console.log(a === require('./a.cjs'), a); const lazy = require('./lazy.cjs'); console.log('before'); console.log(lazy()); for(let i=0;i<2;i++) { try { require('./fail.cjs'); } catch(e) { console.log(e.message); } } console.log(require('./primitive.cjs')); console.log(require('./return.cjs').value, require('./modulevar.cjs').kept);"),
        ("modulevar.cjs", "exports.kept = 3; module = {exports: 99}; exports.after = 4;"),
        ("return.cjs", "#!/usr/bin/env node\n'use strict'; exports.value = 4; console.log(this === exports); return; throw Error('after return');\n//# sourceMappingURL=obsolete.map"),
        ("a.cjs", "exports.first = 1; console.log(this === exports); const b = require('./b.cjs'); exports.old = 3; module.exports = { seen: b.seen }; console.log(require('./a.cjs') === module.exports); exports.lost = 4; exports = { ignored: 5 }; console.log(this.first, module.exports.ignored);"),
        ("b.cjs", "exports.seen = require('./a.cjs').first;"),
        ("lazy.cjs", "module.exports = () => require('./deferred.cjs');"),
        ("deferred.cjs", "console.log('deferred'); module.exports = 7;"),
        ("primitive.cjs", "module.exports = 42; exports.ignored = true;"),
        ("fail.cjs", "globalThis.attempt = (globalThis.attempt || 0) + 1; throw Error('attempt ' + globalThis.attempt);"),
    ]), "entry.cjs");
    assert_eq!(output.backend, Backend::Native);
}

#[test]
fn commonjs_proxy_static_requests_aliases_reexports_and_snapshot_policy() {
    let source = sources(&[
        ("entry.mjs", "import value, {a as renamed, bump} from './dep.cjs'; import {again} from './barrel.mjs'; import * as ns from './dep.cjs'; import bridge from './bridge.cjs'; console.log(renamed, again, ns.a, ns.default === value, bridge === value); bump(); console.log(renamed, ns.default.a, value.a);"),
        ("bridge.cjs", "module.exports = require('./dep.cjs');"),
        ("barrel.mjs", "export {a as again} from './dep.cjs'; export * from './dep.cjs';"),
        ("dep.cjs", "exports.a = 1; exports.bump = () => { exports.a++; };"),
    ]);
    let output = compare(&source, "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    assert_eq!(
        output.cjs_proxies[Path::new("dep.cjs")].1,
        BTreeSet::from(["a".into(), "bump".into(), "default".into()])
    );

    // Deliberately not Node's guessed-exports interop: requested getters and
    // computed names work, __esModule never unwraps default.
    let source = sources(&[
        ("entry.mjs", "import value, {computed as renamed, read, replace} from './dep.cjs'; import * as ns from './dep.cjs'; import rerequire from './bridge.cjs'; console.log(renamed, read, value.default, ns.unknown, ns.default.unknown); replace(); console.log(value.default, ns.default.default, rerequire().default);"),
        ("bridge.cjs", "module.exports = () => require('./dep.cjs');"),
        ("dep.cjs", "module.exports = { __esModule: true, default: 5, ['computed']: 7, unknown: 9, get read() { console.log('getter'); return 11; }, replace() { module.exports = {default: 99}; } };"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    insta::assert_snapshot!(String::from_utf8(actual.stdout).unwrap(), @"
    getter
    7 11 5 9 9
    5 5 99
    ");
}

#[test]
fn commonjs_package_format_survives_owned_graph_and_cache_facts() {
    let source = "module.exports = 17;";
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("dep.js"),
        ModuleFormat::CommonJs,
    )
    .unwrap();
    let graph = SourceGraph::from_owned([OwnedSourceFileInput::from_parsed(
        "dep.js".into(),
        parsed,
        Default::default(),
    )]);
    let policy = EffectPolicy::default();
    let plan = partition(
        &graph,
        &[RootSeed {
            id: RootId::Entry("main".into()),
            evaluation_roots: vec![],
            binding_roots: vec![GraphNode::Segment(SegmentKey::new(0, 0))],
        }],
        &policy,
    )
    .unwrap();
    let mut output = esm(&graph, &plan, &policy, &sources(&[("dep.js", source)])).unwrap();
    assert_eq!(output.backend, Backend::Native);
    output.files.insert(
        "observe.mjs".into(),
        observation(&output, "console.log((await import('./main')).default);"),
    );
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"17\n");
}

#[test]
fn commonjs_proxy_demands_share_one_cached_initializer_across_entries() {
    let source = sources(&[
        ("a.mjs", "export {a} from './shared.cjs';"),
        ("b.mjs", "export {b} from './shared.cjs';"),
        (
            "lazy.cjs",
            "module.exports = () => require('./shared.cjs');",
        ),
        (
            "shared.cjs",
            "console.log('shared once'); const cycle = require('./cycle-a.cjs'); exports.a = {partial: cycle.partial}; exports.b = {};",
        ),
        ("cycle-a.cjs", "exports.phase = 1; const b = require('./cycle-b.cjs'); module.exports = {partial: b.partial};"),
        ("cycle-b.cjs", "exports.partial = require('./cycle-a.cjs').phase;"),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true), ("lazy.cjs", false)];
    let (plan, output) = build(&source, &entries, false);
    let a = NodeId::CommonJsProxy {
        module: "shared.cjs".into(),
        name: Some("a".into()),
    };
    let b = NodeId::CommonJsProxy {
        module: "shared.cjs".into(),
        name: Some("b".into()),
    };
    assert_ne!(plan.chunk_for(&a), plan.chunk_for(&b));
    assert_eq!(
        plan.node_roots[&a],
        BTreeSet::from([RootId::Entry("a.mjs".into())])
    );
    assert_eq!(
        plan.node_roots[&b],
        BTreeSet::from([RootId::Entry("b.mjs".into())])
    );
    for (left, roots) in &plan.node_roots {
        if roots.is_empty() {
            continue;
        }
        for (right, other) in &plan.node_roots {
            if !other.is_empty() {
                assert_eq!(
                    roots == other,
                    plan.chunk_for(left) == plan.chunk_for(right)
                );
            }
        }
        assert!(
            output.chunk_artifacts[&plan.chunk_for(left).unwrap()].contains(&output.members[left])
        );
    }
    assert!(!output.files.keys().any(|path| path.starts_with("proxies")));
    let (_, reversed) = build(&source, &entries, true);
    assert_eq!(output.files, reversed.files);
    let mut files = output.files.clone();
    let observer = r#"
const lazy = await import('./lazy.cjs');
console.log('not initialized');
const [a, b, again] = await Promise.all([import('./a.mjs'), import('./b.mjs'), import('./a.mjs')]);
const common = lazy.default();
console.log(a === again, a.a === common.a, b.b === common.b, common.a.partial);
"#;
    files.insert("observe.mjs".into(), observation(&output, observer));
    let actual = node(&files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    insta::assert_snapshot!(std::str::from_utf8(&actual.stdout).unwrap(), @"
    not initialized
    shared once
    true true true 1
    ");
    let mut baseline = source;
    baseline.insert(
        "observe.mjs".into(),
        r#"
const lazy = await import('./lazy.cjs');
console.log('not initialized');
const [a, b, again] = await Promise.all([import('./a.mjs'), import('./b.mjs'), import('./a.mjs')]);
const common = lazy.default();
console.log(a === again, a.a === common.a, b.b === common.b, common.a.partial);
"#
        .into(),
    );
    let baseline = node(&baseline, Path::new("observe.mjs"));
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert_eq!(actual.stdout, baseline.stdout);
    insta::assert_debug_snapshot!(plan
        .node_roots
        .iter()
        .filter(|(node, _)| matches!(node, NodeId::CommonJsProxy { .. }))
        .collect::<Vec<_>>());
}

#[test]
fn commonjs_mixed_cycles_follow_native_partial_exports_and_deferred_timing() {
    let direct_cycle = sources(&[
        ("entry.cjs", "exports.value = 3; const esm = require('./dep.mjs'); console.log(esm.partial === exports, esm.partial.value);"),
        ("dep.mjs", "import partial from './entry.cjs'; export {partial};"),
    ]);
    let (_, output) = build(&direct_cycle, &[("entry.cjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    for actual in [
        node(&direct_cycle, Path::new("entry.cjs")),
        node(&output.files, &output.entries["entry.cjs"]),
    ] {
        assert!(!actual.status.success());
        assert!(String::from_utf8_lossy(&actual.stderr).contains("ERR_REQUIRE_CYCLE_MODULE"));
    }
    compare(&sources(&[
        ("entry.mjs", "import value from './dep.cjs'; export function read() { return 5; } setTimeout(() => console.log(value.read()), 0);"),
        ("dep.cjs", "module.exports = { read() { return require('./entry.mjs').read(); } };"),
    ]), "entry.mjs");
    compare(&sources(&[
        ("entry.cjs", "exports.value = 3; const esm = require('./dep.mjs'); console.log(esm.read());"),
        ("dep.mjs", "import readPartial from './partial.cjs'; export function read() { return readPartial(); }"),
        ("partial.cjs", "module.exports = () => require('./entry.cjs').value;"),
    ]), "entry.cjs");
    let source = sources(&[
        ("entry.mjs", "import './dep.cjs'; export const value = 3;"),
        (
            "dep.cjs",
            "console.log('before native cycle error'); require('./entry.mjs');",
        ),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    let baseline = node(&source, Path::new("entry.mjs"));
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(!baseline.status.success() && !actual.status.success());
    assert_eq!(actual.stdout, baseline.stdout);
    for result in [&baseline, &actual] {
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("ERR_REQUIRE_CYCLE_MODULE"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn commonjs_native_getters_follow_demand_seams_and_static_order() {
    let source = sources(&[
        ("a.mjs", "export {a} from './dep.cjs';"),
        ("b.mjs", "export {b} from './dep.cjs';"),
        ("dep.cjs", "console.log('factory'); const __gf_cjs_0_reserved = 1; module.exports = {get a() {console.log('a getter');return __gf_cjs_0_reserved;}, get b() {console.log('b getter');return 2;}};"),
    ]);
    let (_, mut output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    assert_eq!(output.backend, Backend::Native);
    let observer = observation(&output, "const a = await import('./a.mjs'); console.log('a ready',a.a); const b = await import('./b.mjs'); console.log('b ready',b.b); console.log(a === await import('./a.mjs'));");
    output.files.insert("observe.mjs".into(), observer);
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"factory\na getter\na ready 1\nb getter\nb ready 2\ntrue\n"
    );

    let source = sources(&[
        ("entry.mjs", "import {z} from './dep.cjs'; import './sibling.mjs'; import {a} from './dep.cjs'; console.log(a,z);"),
        ("dep.cjs", "console.log('factory'); Object.defineProperties(exports,{z:{get(){console.log('z getter');return 2;}}, a:{get(){console.log('a getter');return 1;}}, unused:{get(){throw Error('unused');}}});"),
        ("sibling.mjs", "console.log('sibling');"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"factory\na getter\nz getter\nsibling\n1 2\n"
    );
    let source = sources(&[
        ("entry.mjs", "import value from './dep.cjs'; import {a} from './barrel.mjs'; console.log('entry',value.a,a);"),
        ("barrel.mjs", "import './sibling.mjs'; export {a} from './dep.cjs';"),
        ("sibling.mjs", "console.log('sibling');"),
        ("dep.cjs", "console.log('factory'); Object.defineProperty(exports,'a',{get(){console.log('a getter');return 1;}});"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"factory\nsibling\na getter\na getter\nentry 1 1\n"
    );
}

#[test]
fn commonjs_unused_binding_keeps_evaluation_without_requesting_its_getter() {
    let source = sources(&[
            ("entry.mjs", "import {unused} from './dep.cjs'; console.log('entry');"),
            ("dep.cjs", "console.log('dependency'); Object.defineProperty(exports, 'unused', {get() { throw Error('unused getter executed'); }});"),
        ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(
        output.cjs_proxies[Path::new("dep.cjs")].1,
        BTreeSet::from(["default".into()])
    );
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"dependency\nentry\n");
    let source = sources(&[
        ("entry.mjs", "import {used, unused} from './dep.cjs'; console.log('entry',used);"),
        ("dep.cjs", "console.log('dependency'); exports.used = 3; Object.defineProperty(exports, 'unused', {get() { throw Error('unused getter executed'); }});"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"dependency\nentry 3\n");
}

#[test]
fn commonjs_namespace_conservatively_captures_runtime_keys_and_live_properties() {
    let source = sources(&[
        ("entry.mjs", "import * as ns from './dep.cjs'; import {box} from './barrel.mjs'; export {ns}; const key = 'dynamic' + 'Name'; console.log(ns === box, Object.isFrozen(ns), ns[key], Object.keys(ns).sort().join(',')); console.log(ns.read); ns.bump(); console.log(ns[key], ns.read, ns.added, ns.default.added);"),
        ("barrel.mjs", "export * as box from './dep.cjs';"),
        ("dep.cjs", "let value = 1; module.exports = { ['dynamic' + 'Name']: value, get read() { console.log('getter'); return value; }, bump() { value++; module.exports.dynamicName = value; module.exports.added = 9; } };"),
    ]);
    let (_, mut output) = build(&source, &[("entry.mjs", true), ("dep.cjs", false)], false);
    assert_eq!(
        output.cjs_proxies[Path::new("dep.cjs")].1,
        BTreeSet::from(["default".into()])
    );
    let observer = observation(&output, "const entry = await import('./entry.mjs'); console.log(entry.ns === await import('./dep.cjs'));");
    output.files.insert("observe.mjs".into(), observer);
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    insta::assert_snapshot!(String::from_utf8(actual.stdout).unwrap(), @"
    true true 1 bump,default,dynamicName,read
    getter
    1
    getter
    2 2 undefined 9
    true
    ");
}

#[test]
fn native_shared_source_scc_preserves_both_entry_orders() {
    let source = sources(&[
        ("a.mjs", "import './b.mjs'; console.log('a');"),
        ("b.mjs", "import './a.mjs'; console.log('b');"),
        ("x.mjs", "import './a.mjs'; console.log('x');"),
        ("y.mjs", "import './b.mjs'; console.log('y');"),
    ]);
    let baselines: BTreeMap<_, _> = ["x.mjs", "y.mjs"]
        .into_iter()
        .map(|entry| {
            let baseline = node(&source, Path::new(entry));
            assert!(baseline.status.success());
            assert!(baseline.stderr.is_empty());
            (entry, baseline.stdout)
        })
        .collect();
    let mut mismatches = Vec::new();
    for entries in [
        [("x.mjs", true), ("y.mjs", true)],
        [("y.mjs", true), ("x.mjs", true)],
    ] {
        for reverse_graph in [false, true] {
            let (_, output) = build(&source, &entries, reverse_graph);
            assert_eq!(output.backend, Backend::Native);
            for (entry, _) in entries {
                let actual = node(&output.files, &output.entries[entry]);
                assert!(
                    actual.status.success(),
                    "{}",
                    String::from_utf8_lossy(&actual.stderr)
                );
                assert!(actual.stderr.is_empty());
                if actual.stdout != baselines[entry] {
                    mismatches.push(format!(
                        "roots={entries:?}, reverse_graph={reverse_graph}, entry={entry}: expected {:?}, got {:?}",
                        String::from_utf8_lossy(&baselines[entry]),
                        String::from_utf8_lossy(&actual.stdout),
                    ));
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn native_synchronous_diamonds_and_cycles_preserve_native_phases() {
    for source in [
        sources(&[
            (
                "entry.mjs",
                "import {left} from './left.mjs'; import {right} from './right.mjs'; let todo; const run = () => { todo = left() + right(); }; run(); console.log(todo, left.name, right.name, run.name);",
            ),
            (
                "left.mjs",
                "import {State} from './shared.mjs'; const template = 'left'; console.log(template); export const left = () => new State(template).value;",
            ),
            (
                "right.mjs",
                "import {State} from './shared.mjs'; const template = 'right'; console.log(template); export const right = () => new State(template).value;",
            ),
            (
                "shared.mjs",
                "console.log('shared'); export class State { constructor(todo) { this.value = todo; } }",
            ),
        ]),
        sources(&[
            (
                "entry.mjs",
                "import {read, value} from './a.mjs'; console.log(read(), value, read.name);",
            ),
            (
                "a.mjs",
                "import {initial} from './b.mjs'; export var value = initial + 1; export function read() { return value; }",
            ),
            (
                "b.mjs",
                "import {read, value} from './a.mjs'; console.log(typeof read, value, read()); export var initial = 4;",
            ),
        ]),
    ] {
        let output = compare(&source, "entry.mjs");
        assert_eq!(output, build(&source, &[("entry.mjs", false)], true).1);

        let mut multiple = source;
        multiple.insert("other.mjs".into(), "export const other = 1;".into());
        let (_, output) = build(
            &multiple,
            &[("entry.mjs", true), ("other.mjs", true)],
            false,
        );
        assert_ne!(output.entries["entry.mjs"], output.entries["other.mjs"]);
        assert_eq!(
            node(&output.files, &output.entries["entry.mjs"]).stdout,
            node(&multiple, Path::new("entry.mjs")).stdout
        );
    }
}

#[test]
fn native_async_regions_keep_lazy_roots_and_shared_dependencies_separate() {
    let source = sources(&[
        (
            "entry.mjs",
            "export const left = () => import('./left.mjs'); export const right = () => import('./right.mjs'); console.log('entry'); left().then(async first => { console.log(first.value); console.log((await right()).value); });",
        ),
        (
            "left.mjs",
            "import {shared} from './shared.mjs'; import one from './one.mjs'; import two from './two.mjs'; console.log('lazy-left'); export const value = shared.value + one + two;",
        ),
        (
            "right.mjs",
            "import {shared} from './shared.mjs'; import three from './three.mjs'; console.log('lazy-right'); export const value = shared.value + three;",
        ),
        (
            "shared.mjs",
            "console.log('shared'); export const shared = Object.freeze({value: 10});",
        ),
        (
            "one.mjs",
            "class One {} console.log(One.name); export default 1;",
        ),
        (
            "two.mjs",
            "class Two {} console.log(Two.name); export default 2;",
        ),
        (
            "three.mjs",
            "class Three {} console.log(Three.name); export default 3;",
        ),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", true)], false);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, node(&source, Path::new("entry.mjs")).stdout);
    let owner = |module: &str| &output.members[&NodeId::Module(module.into())];
    assert_ne!(owner("left.mjs"), owner("one.mjs"));
    assert_ne!(owner("left.mjs"), owner("two.mjs"));
    assert_ne!(owner("right.mjs"), owner("three.mjs"));
    assert_ne!(owner("left.mjs"), owner("right.mjs"));
    assert_ne!(owner("left.mjs"), owner("shared.mjs"));
    assert_ne!(owner("right.mjs"), owner("shared.mjs"));
    let entry = &output.files[&output.entries["entry.mjs"]];
    assert!(!entry.contains("lazy-left") && !entry.contains("lazy-right"));
    assert_eq!(output, build(&source, &[("entry.mjs", true)], true).1);
}

#[test]
fn native_async_regions_preserve_effects_before_a_shared_dependency() {
    let source = sources(&[
        (
            "a.mjs",
            "import './early.mjs'; import './late.mjs'; console.log('a');",
        ),
        ("b.mjs", "import './shared.mjs'; console.log('b');"),
        ("early.mjs", "console.log('early');"),
        ("late.mjs", "import './shared.mjs'; console.log('late');"),
        ("shared.mjs", "console.log('shared');"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    for entry in ["a.mjs", "b.mjs"] {
        let baseline = node(&source, Path::new(entry));
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
    }
}

#[test]
fn native_chunks_share_live_state_and_concatenate_private_chains() {
    let source = sources(&[
        (
            "state.mjs",
            "globalThis.sharedInitializations = (globalThis.sharedInitializations || 0) + 1; export const sharedObject = {}; export let counter = 0; export function increment() { counter++; } export function strictThis() { return this; } const privateSentinel = 'state-private-body'; console.log(privateSentinel);",
        ),
        ("pass.mjs", "export * from './state.mjs';"),
        (
            "private.mjs",
            "import {sharedObject, counter, increment, strictThis} from './pass.mjs'; export {sharedObject as object, counter, increment}; console.log('private-chain-body', strictThis() === undefined); try { counter = 99; } catch (error) { console.log(error.name); }",
        ),
        (
            "a.mjs",
            "export {object, counter, increment} from './private.mjs';",
        ),
        (
            "b.mjs",
            "export {sharedObject as other, counter as live} from './pass.mjs';",
        ),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    assert_eq!(output.backend, Backend::Native);
    let owner = |module: &str| &output.members[&NodeId::Module(module.into())];
    assert_ne!(owner("a.mjs"), owner("private.mjs"));
    assert_eq!(owner("pass.mjs"), owner("state.mjs"));
    assert_ne!(owner("a.mjs"), owner("state.mjs"));
    assert!(!output.modules.contains_key(Path::new("private.mjs")));
    assert!(!output.modules.contains_key(Path::new("pass.mjs")));
    assert!(output.files[owner("private.mjs")].contains("private-chain-body"));
    assert!(output.files[owner("private.mjs")].contains("import {"));
    assert!(!output.files[owner("a.mjs")].contains("from './private.mjs'"));
    let check = |a: &Path, b: &Path| {
        format!(
            "import * as a from {}; import * as b from {}; console.log(Object.keys(a).join(','), Object.keys(b).join(',')); a.increment(); console.log(a.object === b.other, a.counter, b.live, globalThis.sharedInitializations);",
            quote(&format!("./{}", a.display())),
            quote(&format!("./{}", b.display()))
        )
    };
    let mut native = source.clone();
    native.insert(
        "check.mjs".into(),
        check(Path::new("a.mjs"), Path::new("b.mjs")),
    );
    let baseline = node(&native, Path::new("check.mjs"));
    let mut files = output.files.clone();
    files.insert(
        "check.mjs".into(),
        check(&output.entries["a.mjs"], &output.entries["b.mjs"]),
    );
    let actual = node(&files, Path::new("check.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, baseline.stdout);
    assert_eq!(actual.stdout, b"state-private-body\nprivate-chain-body true\nTypeError\ncounter,increment,object live,other\ntrue 1 1 1\n");
    assert_eq!(
        output,
        build(&source, &[("a.mjs", true), ("b.mjs", true)], true).1
    );
}

#[test]
fn native_chunks_preserve_entry_dependent_static_effect_order() {
    let source = sources(&[
        (
            "a.mjs",
            "import './left.mjs'; import './right.mjs'; console.log('a');",
        ),
        (
            "b.mjs",
            "import './right.mjs'; import './left.mjs'; console.log('b');",
        ),
        ("left.mjs", "console.log('left');"),
        ("right.mjs", "console.log('right');"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    assert_eq!(output.backend, Backend::Native);
    for entry in ["a.mjs", "b.mjs"] {
        let native = node(&source, Path::new(entry));
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, native.stdout);
    }
}

#[test]
fn native_public_namespaces_keep_private_deferred_dependencies_addressable() {
    for (dependency, expected, separate_private_reader) in [
        (
            "const Base = class {}; export const value = class Self extends (() => Base)() { static own() { return Self; } }; export function read() { return [value.name, value.own() === value, Object.getPrototypeOf(value) === Base]; }",
            "[\"Self\",true,true]\nread,value\n",
            false,
        ),
        (
            "let value = 0; if (true) { globalThis.capture(() => [value, after, hoisted()]); value = 2; } function hoisted() { return value; } export const after = 3; export function read() { return [value, after, hoisted()]; }",
            "ReferenceError\n[2,3,2]\nafter,read\n",
            false,
        ),
        (
            "const secret = (console.log('initialize'), {value: 7}); export function read() { return secret.value; }",
            "initialize\n7\nread\n",
            true,
        ),
    ] {
        let source = sources(&[
            (
                "entry.mjs",
                "import * as ns from './dep.mjs'; export function observe() { console.log(JSON.stringify(ns.read())); console.log(Object.keys(ns).join(',')); }",
            ),
            ("other.mjs", "export {read} from './dep.mjs';"),
            ("dep.mjs", dependency),
        ]);
        let mut private_owners = Vec::new();
        for reverse in [false, true] {
            let entries = if reverse {
                [("other.mjs", true), ("entry.mjs", true)]
            } else {
                [("entry.mjs", true), ("other.mjs", true)]
            };
            let (plan, output) = build(&source, &entries, reverse);
            assert_eq!(output.backend, Backend::Native);
            let observer = format!(
                "globalThis.capture = f => {{ try {{ f(); }} catch (e) {{ console.log(e.name); }} }}; {} (await import('./entry.mjs')).observe();",
                if reverse { "await import('./other.mjs');" } else { "" }
            );
            let mut baseline = source.clone();
            baseline.insert("observe.mjs".into(), observer.clone());
            let baseline = node(&baseline, Path::new("observe.mjs"));
            assert!(baseline.status.success());
            assert!(baseline.stderr.is_empty());
            assert_eq!(baseline.stdout, expected.as_bytes());
            let mut actual = output.files.clone();
            actual.insert("observe.mjs".into(), observation(&output, &observer));
            let actual = node(&actual, Path::new("observe.mjs"));
            assert!(actual.status.success(), "{}", String::from_utf8_lossy(&actual.stderr));
            assert!(actual.stderr.is_empty());
            assert_eq!(actual.stdout, baseline.stdout);
            if separate_private_reader {
                let graph = graph(&source, reverse);
                let file = graph.file_id(Path::new("dep.mjs")).unwrap();
                let secret = SegmentKey::new(file, 0);
                let reader = SegmentKey::new(file, 1);
                assert!(plan.retention.effects.iter().any(|effect| {
                    effect.key == secret && effect.summary.is_effectful()
                }));
                assert!(plan.retention.edges.iter().any(|edge| {
                    edge.kind == crate::RetentionEdgeKind::Binding
                        && edge.from == GraphNode::Segment(reader)
                        && edge.to == GraphNode::Segment(secret)
                }));
                let access = plan.retention.execution.accesses.iter()
                    .find(|access| access.segment == reader && access.declaration == secret)
                    .unwrap();
                let placement = &graph.file_facts(file).unwrap().emission.segments[1].placement;
                assert!(placement.instantiable);
                assert!(!placement.eager_reads.contains(&access.reference));
                assert!(!plan.retention.execution.constraints.iter().any(|constraint| {
                    (constraint.before == secret && constraint.after == reader)
                        || (constraint.before == reader && constraint.after == secret)
                }));
                let owner = |index| &output.members[&NodeId::Segment {
                    module: "dep.mjs".into(),
                    index,
                }];
                private_owners.push((owner(0).clone(), owner(1).clone()));
            }
        }
        // Execution pins the initializer; both entries demand the reader.
        // Privacy is not a reason to collapse their physical owners.
        for (initializer, reader) in private_owners {
            assert_ne!(initializer, reader,
                "private binding availability must cross owners without widening the public namespace");
        }
    }
}

#[test]
fn native_connections_separate_private_transport_from_all_namespace_aliases() {
    let source = sources(&[
        ("dep.mjs", "const secret = (console.log('initialize'), {value: 7}); const spare = (console.log('spare'), 9); export let current = 1; export {current as live}; export default current; export function bump() { current++; } export function read() { return secret.value; }"),
        ("entry.mjs", "import * as ns from './dep.mjs'; import {current} from './dep.mjs'; export {ns}; export const load = () => import('./dep.mjs'); export function invalid() { current = 9; }"),
        ("other.mjs", "export {read} from './dep.mjs';"),
        ("loader.cjs", "module.exports = () => import('./dep.mjs');"),
    ]);
    let observer = "const entry = await import('./entry.mjs'); const direct = await import('./dep.mjs'); const dynamic = await entry.load(); const loaded = await (await import('./loader.cjs')).default(); console.log(entry.ns === direct, direct === dynamic, direct === loaded, Object.keys(dynamic).join(',')); direct.bump(); try { entry.invalid(); } catch (error) { console.log(error.name); } console.log(direct.default, direct.current, direct.live, direct.read(), (await import('./other.mjs')).read === direct.read);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let baseline = node(&baseline, Path::new("observe.mjs"));
    assert!(baseline.status.success());
    assert!(baseline.stderr.is_empty());
    assert_eq!(baseline.stdout, b"initialize\nspare\ntrue true true bump,current,default,live,read\nTypeError\n1 2 2 7 true\n");
    for reverse in [false, true] {
        let (plan, output) = build(
            &source,
            &[
                ("dep.mjs", true),
                ("entry.mjs", true),
                ("other.mjs", true),
                ("loader.cjs", true),
            ],
            reverse,
        );
        let body = &output.members[&NodeId::Segment {
            module: "dep.mjs".into(),
            index: 0,
        }];
        let reader = &output.members[&NodeId::Segment {
            module: "dep.mjs".into(),
            index: 6,
        }];
        assert_ne!(body, reader);
        assert_ne!(body, &output.modules[Path::new("dep.mjs")]);
        let module = NodeId::Module("dep.mjs".into());
        assert_eq!(body, &output.members[&module]);
        let projected = &output.chunk_artifacts[&plan.chunk_for(&module).unwrap()];
        assert!(projected.contains(body));
        assert!(projected.contains(&output.modules[Path::new("dep.mjs")]));
        let mut actual = output.files.clone();
        actual.insert("observe.mjs".into(), observation(&output, observer));
        let observed = node(&actual, Path::new("observe.mjs"));
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert!(observed.stderr.is_empty());
        assert_eq!(observed.stdout, baseline.stdout);
        actual.insert("inspect.mjs".into(), format!(
            "const body = await import({}); const publicNs = await import({}); const internal = Object.keys(body).filter(key => !Object.hasOwn(publicNs, key)); console.log(internal.length, body[internal[0]].value);",
            serde_json::to_string(&format!("./{}", body.to_string_lossy().replace('\\', "/"))).unwrap(),
            serde_json::to_string(&format!("./{}", output.modules[Path::new("dep.mjs")].to_string_lossy().replace('\\', "/"))).unwrap(),
        ));
        let inspected = node(&actual, Path::new("inspect.mjs"));
        assert!(
            inspected.status.success(),
            "{}",
            String::from_utf8_lossy(&inspected.stderr)
        );
        assert_eq!(inspected.stdout, b"initialize\nspare\n1 7\n");
    }
}

#[test]
fn native_connections_preserve_import_assignment_inference() {
    let source = sources(&[
        ("dep.mjs", "export let value = (console.log('init'), 1);"),
        ("entry.mjs", "import {value} from './dep.mjs'; export function run() { try { value = () => {}; } catch (error) { console.log(error.name); } }"),
    ]);
    let observer = "const entry = await import('./entry.mjs'); entry.run();";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let baseline = node(&baseline, Path::new("observe.mjs"));
    assert!(baseline.status.success());
    assert_eq!(baseline.stdout, b"init\nTypeError\n");
    for reverse in [false, true] {
        let (_, output) = build(&source, &[("dep.mjs", true), ("entry.mjs", true)], reverse);
        let mut actual = output.files.clone();
        actual.insert("observe.mjs".into(), observation(&output, observer));
        let observed = node(&actual, Path::new("observe.mjs"));
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert_eq!(observed.stdout, baseline.stdout);
    }
}

#[test]
fn native_connections_preserve_readonly_import_rhs_names() {
    for (declaration, clause, local, keys, prefix) in [
        ("export let value = 1;", "{value}", "value", "value", ""),
        (
            "export let value = 1;",
            "{value as local}",
            "local",
            "value",
            "",
        ),
        ("export default 1;", "value", "value", "default", ""),
        ("export let value = 1;", "* as value", "value", "value", ""),
        (
            "export let value = (console.log('init'), 1);",
            "* as value",
            "value",
            "value",
            "init\n",
        ),
    ] {
        let source = sources(&[
            ("dep.mjs", declaration),
            (
                "entry.mjs",
                &format!(
                    "import {clause} from './dep.mjs'; export function run() {{ try {{ {local} = () => {{}}; }} catch (error) {{ console.log('arrow', error.name); }} try {{ {local} = class {{ static {{ console.log(this.name); }} }}; }} catch (error) {{ console.log('class', error.name); }} }}"
                ),
            ),
        ]);
        let observer = "const entry = await import('./entry.mjs'); entry.run(); const dep = await import('./dep.mjs'); console.log(Object.keys(dep).join(','), dep.value ?? dep.default);";
        let mut baseline = source.clone();
        baseline.insert("observe.mjs".into(), observer.into());
        let baseline = node(&baseline, Path::new("observe.mjs"));
        assert!(
            baseline.status.success(),
            "{}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        assert!(baseline.stderr.is_empty());
        assert_eq!(
            baseline.stdout,
            format!("{prefix}arrow TypeError\n{local}\nclass TypeError\n{keys} 1\n").as_bytes(),
        );
        for reverse in [false, true] {
            let (_, output) = build(&source, &[("dep.mjs", true), ("entry.mjs", true)], reverse);
            let mut actual = output.files.clone();
            actual.insert("observe.mjs".into(), observation(&output, observer));
            let observed = node(&actual, Path::new("observe.mjs"));
            assert!(
                observed.status.success(),
                "{}",
                String::from_utf8_lossy(&observed.stderr)
            );
            assert!(observed.stderr.is_empty());
            assert_eq!(observed.stdout, baseline.stdout);
        }
    }
}

#[test]
fn native_connections_split_incompatible_import_and_declaration_names() {
    let source = sources(&[
        ("dep.mjs", "export let value = 1;"),
        (
            "writer.mjs",
            "import {value as collision} from './dep.mjs'; export function run() { try { collision = class { static { console.log(this.name); } }; } catch (error) { console.log(error.name); } }",
        ),
        (
            "reader.mjs",
            "const collision = 7; export function read() { return collision; }",
        ),
        (
            "entry.mjs",
            "export {run} from './writer.mjs'; export {read} from './reader.mjs';",
        ),
    ]);
    let observer = "const entry = await import('./entry.mjs'); entry.run(); console.log(entry.read(), (await import('./dep.mjs')).value);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let baseline = node(&baseline, Path::new("observe.mjs"));
    assert!(baseline.status.success());
    assert_eq!(baseline.stdout, b"collision\nTypeError\n7 1\n");
    for reverse in [false, true] {
        let (_, output) = build(&source, &[("dep.mjs", true), ("entry.mjs", true)], reverse);
        let owner = |module: &str, index| {
            &output.members[&NodeId::Segment {
                module: module.into(),
                index,
            }]
        };
        assert_ne!(owner("writer.mjs", 1), owner("reader.mjs", 0));
        let mut actual = output.files.clone();
        actual.insert("observe.mjs".into(), observation(&output, observer));
        let observed = node(&actual, Path::new("observe.mjs"));
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert!(observed.stderr.is_empty());
        assert_eq!(observed.stdout, baseline.stdout);
    }
}

#[test]
fn native_connections_remove_obligation_free_private_cycles() {
    for indirect in [false, true] {
        let mut source = sources(&[
            (
                "a.mjs",
                "import {b} from './b.mjs'; export function a(n) { return n ? b(n - 1) : 'a'; }",
            ),
            (
                "b.mjs",
                "import {a} from './a.mjs'; export function b(n) { return n ? a(n - 1) : 'b'; }",
            ),
            (
                "entry.mjs",
                "export {a} from './a.mjs'; export {b} from './b.mjs';",
            ),
            (
                "other.mjs",
                "export {a} from './a.mjs'; export {b} from './b.mjs';",
            ),
        ]);
        if indirect {
            source.insert("b.mjs".into(), "import {a} from './bridge.mjs'; export function b(n) { return n ? a(n - 1) : 'b'; }".into());
            source.insert("bridge.mjs".into(), "export {a} from './a.mjs';".into());
        }
        for reverse in [false, true] {
            let (_, output) = build(
                &source,
                &[("entry.mjs", true), ("other.mjs", true)],
                reverse,
            );
            let owner = &output.members[&NodeId::Segment {
                module: "a.mjs".into(),
                index: 1,
            }];
            assert_eq!(
                owner,
                &output.members[&NodeId::Segment {
                    module: "b.mjs".into(),
                    index: 1,
                }]
            );
            assert_eq!(
                output.files.keys().cloned().collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    owner.clone(),
                    output.entries["entry.mjs"].clone(),
                    output.entries["other.mjs"].clone(),
                ]),
                "body-free private connectors must not impose source-module seams",
            );
            assert!(!output.modules.contains_key(Path::new("a.mjs")));
            assert!(!output.modules.contains_key(Path::new("b.mjs")));
            for first in ["entry", "other"] {
                let second = if first == "entry" { "other" } else { "entry" };
                let observer = format!(
                    "const first = await import('./{first}.mjs'); const second = await import('./{second}.mjs'); console.log(first !== second, first.a === second.a, first.b === second.b, Object.keys(first).join(','), Object.keys(second).join(','), first.a.name, first.b.name, first.a(2), first.b(2), first.a(3), first.b(3));"
                );
                let mut baseline = source.clone();
                baseline.insert("observe.mjs".into(), observer.clone());
                let baseline = node(&baseline, Path::new("observe.mjs"));
                assert!(baseline.status.success());
                assert!(baseline.stderr.is_empty());
                assert_eq!(baseline.stdout, b"true true true a,b a,b a b a b b a\n");
                let mut actual = output.files.clone();
                actual.insert("observe.mjs".into(), observation(&output, &observer));
                let observed = node(&actual, Path::new("observe.mjs"));
                assert!(
                    observed.status.success(),
                    "{}",
                    String::from_utf8_lossy(&observed.stderr)
                );
                assert!(observed.stderr.is_empty());
                assert_eq!(observed.stdout, baseline.stdout);
            }
        }
    }
}

#[test]
fn native_connections_preserve_empty_cycles_with_ordered_effectful_descendants() {
    let source = sources(&[
        ("entry.mjs", "import './x.mjs'; console.log('entry');"),
        ("other.mjs", "import './y.mjs'; console.log('other');"),
        ("x.mjs", "import './y.mjs'; import './first.mjs';"),
        ("y.mjs", "import './x.mjs'; import './second.mjs';"),
        ("first.mjs", "console.log('first');"),
        ("second.mjs", "console.log('second');"),
    ]);
    for reverse in [false, true] {
        let (_, output) = build(
            &source,
            &[("entry.mjs", true), ("other.mjs", true)],
            reverse,
        );
        for (first, second, expected) in [
            ("entry", "other", "second\nfirst\nentry\nother\n"),
            ("other", "entry", "first\nsecond\nother\nentry\n"),
        ] {
            let observer =
                format!("await import('./{first}.mjs'); await import('./{second}.mjs');");
            let mut baseline = source.clone();
            baseline.insert("observe.mjs".into(), observer.clone());
            let baseline = node(&baseline, Path::new("observe.mjs"));
            assert!(baseline.status.success());
            assert_eq!(baseline.stdout, expected.as_bytes());
            let mut actual = output.files.clone();
            actual.insert("observe.mjs".into(), observation(&output, &observer));
            let observed = node(&actual, Path::new("observe.mjs"));
            assert!(
                observed.status.success(),
                "{}",
                String::from_utf8_lossy(&observed.stderr)
            );
            assert!(observed.stderr.is_empty());
            assert_eq!(observed.stdout, baseline.stdout);
        }
    }
}

#[test]
fn native_connections_preserve_connector_cycle_failure_across_entrances() {
    for fail_on in ["first", "second"] {
        let source = sources(&[
            ("entry.mjs", "import './x.mjs'; console.log('entry');"),
            ("other.mjs", "import './y.mjs'; console.log('other');"),
            ("x.mjs", "import './y.mjs'; import './first.mjs';"),
            ("y.mjs", "import './x.mjs'; import './second.mjs';"),
            ("first.mjs", "import './first-effect.mjs';"),
            ("second.mjs", "import './second-effect.mjs';"),
            (
                "first-effect.mjs",
                "console.log('first'); if (globalThis.failOn === 'first') throw Error('first');",
            ),
            (
                "second-effect.mjs",
                "console.log('second'); if (globalThis.failOn === 'second') throw Error('second');",
            ),
        ]);
        for reverse in [false, true] {
            let (_, output) = build(
                &source,
                &[("entry.mjs", true), ("other.mjs", true)],
                reverse,
            );
            for (first, second) in [("entry", "other"), ("other", "entry")] {
                let observer = format!(
                    "globalThis.failOn = '{fail_on}'; let saved; try {{ await import('./{first}.mjs'); }} catch (error) {{ saved = error; console.log('caught', error.message); }} try {{ await import('./{second}.mjs'); }} catch (error) {{ console.log('same failure', error === saved); }}"
                );
                let mut baseline = source.clone();
                baseline.insert("observe.mjs".into(), observer.clone());
                let baseline = node(&baseline, Path::new("observe.mjs"));
                assert!(baseline.status.success());
                assert!(baseline.stderr.is_empty());
                let mut actual = output.files.clone();
                actual.insert("observe.mjs".into(), observation(&output, &observer));
                let observed = node(&actual, Path::new("observe.mjs"));
                assert!(
                    observed.status.success(),
                    "{}",
                    String::from_utf8_lossy(&observed.stderr)
                );
                assert!(observed.stderr.is_empty());
                assert_eq!(observed.stdout, baseline.stdout);
            }
        }
    }
}

#[test]
fn native_connections_keep_same_owner_dependencies_local() {
    let source = sources(&[
        (
            "dep.mjs",
            "const secret = 7; export function read() { return secret; }",
        ),
        ("entry.mjs", "export {read} from './dep.mjs';"),
        ("other.mjs", "export {read} from './dep.mjs';"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", true), ("other.mjs", true)], false);
    let owner = |index| {
        &output.members[&NodeId::Segment {
            module: "dep.mjs".into(),
            index,
        }]
    };
    assert_eq!(owner(0), owner(1));
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), format!(
        "const ns = await import({}); console.log(Object.keys(ns).length, Object.values(ns)[0]());",
        serde_json::to_string(&format!("./{}", owner(0).to_string_lossy().replace('\\', "/"))).unwrap(),
    ));
    let observed = node(&actual, Path::new("observe.mjs"));
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert_eq!(observed.stdout, b"1 7\n");
}

#[test]
fn native_segment_schedule_preserves_abrupt_export_initialization_state() {
    let source = sources(&[
        (
            "a.mjs",
            "import './capture.mjs'; export const before = 1; throw Error('stop'); export const after = 2; export function read() { let value; try { value = after; } catch (error) { value = error.name; } return [before, value].join(' '); }",
        ),
        (
            "capture.mjs",
            "import {read} from './a.mjs'; globalThis.savedRead = read;",
        ),
        ("b.mjs", "export const independent = 3;"),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true)];
    let (_, output) = build(&source, &entries, false);
    let observer = "try { await import('./a.mjs'); } catch (error) { console.log(error.message); } console.log(globalThis.savedRead()); console.log((await import('./b.mjs')).independent);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let baseline = node(&baseline, Path::new("observe.mjs"));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, baseline.stdout);
    insta::assert_snapshot!(String::from_utf8(actual.stdout).unwrap(), @"
    stop
    1 ReferenceError
    3
    ");
}

#[test]
fn native_segment_schedule_preserves_escaped_namespace_initialization_on_failure() {
    for (entry, x, y, capture, expected) in [
        (
            "import './capture.mjs'; export {x} from './x.mjs'; export {y} from './y.mjs';",
            "export const x = 1; console.log('x');",
            "export const y = 2; console.log('y');",
            "import './y.mjs'; import * as ns from './entry.mjs'; globalThis.saved = ns; throw Error('stop');",
            "y\nstop\nReferenceError\n",
        ),
        (
            "import {x} from './x.mjs'; import './capture.mjs'; export {x};",
            "export const x = 1;",
            "",
            "import * as ns from './entry.mjs'; globalThis.saved = ns; throw Error('stop');",
            "stop\n1\n",
        ),
    ] {
        let source = sources(&[
            ("entry.mjs", entry),
            ("x.mjs", x),
            ("y.mjs", y),
            ("capture.mjs", capture),
        ]);
        let observer = "try { await import('./entry.mjs'); } catch (e) { console.log(e.message); } try { console.log(saved.x); } catch (e) { console.log(e.name); }";
        let mut baseline = source.clone();
        baseline.insert("observe.mjs".into(), observer.into());
        let baseline = node(&baseline, Path::new("observe.mjs"));
        assert!(baseline.status.success());
        assert!(baseline.stderr.is_empty());
        assert_eq!(baseline.stdout, expected.as_bytes());
        for reverse in [false, true] {
            let (_, output) = build(&source, &[("entry.mjs", true)], reverse);
            assert_eq!(output.backend, Backend::Native);
            let mut actual = output.files.clone();
            actual.insert("observe.mjs".into(), observation(&output, observer));
            let actual = node(&actual, Path::new("observe.mjs"));
            assert!(actual.status.success(), "{}", String::from_utf8_lossy(&actual.stderr));
            assert!(actual.stderr.is_empty());
            assert_eq!(actual.stdout, baseline.stdout);
        }
    }
}

#[test]
fn native_segment_schedule_preserves_deferred_class_and_getter_snapshots() {
    let source = sources(&[
        (
            "a.mjs",
            "export {snapshot, instance, later, getter, read} from './shared.mjs';",
        ),
        ("b.mjs", "export {instance, read} from './shared.mjs';"),
        (
            "shared.mjs",
            "let state = 0; export const snapshot = state; class Box { field = (() => { state++; return state; })(); } const Alias = Box; export const instance = new Alias(); export const later = state; const object = {get value() { state++; return state; }}; export const getter = object.value; export function read() { return state; }",
        ),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true)];
    let (_, output) = build(&source, &entries, false);
    let observer = "const a = await import('./a.mjs'); const b = await import('./b.mjs'); console.log(a.snapshot, a.instance.field, a.later, a.getter, a.read(), a.instance === b.instance, a.read === b.read);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        node(&baseline, Path::new("observe.mjs")).stdout
    );
    insta::assert_snapshot!(String::from_utf8(actual.stdout).unwrap(), @"0 1 1 2 2 true true");
}

#[test]
fn native_pure_segment_schedule_colocates_independent_and_shared_components() {
    let source = sources(&[
        (
            "a.mjs",
            "export {left, token, group} from './shared.mjs'; export {first} from './left.mjs'; export {safe, early, late} from './effect.mjs'; export {otherEarly} from './effect2.mjs';",
        ),
        (
            "b.mjs",
            "export {right, token, group} from './shared.mjs'; export {second} from './right.mjs'; export {early} from './effect.mjs'; export {otherEarly} from './effect2.mjs';",
        ),
        (
            "shared.mjs",
            "export const left = 1; export const right = 2; export const token = {}; export function group() { return 3; }",
        ),
        ("left.mjs", "const base = 5; export const first = base;"),
        ("right.mjs", "export const second = 6;"),
        (
            "effect.mjs",
            "export const early = {}; export function safe() { return 7; } console.log('effect'); export const late = 8;",
        ),
        (
            "effect2.mjs",
            "export const otherEarly = {}; console.log('effect2');",
        ),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true)];
    let mut pure = source.clone();
    pure.remove(Path::new("effect.mjs"));
    pure.remove(Path::new("effect2.mjs"));
    pure.insert(
        "a.mjs".into(),
        "export {left, token, group} from './shared.mjs'; export {first} from './left.mjs';".into(),
    );
    pure.insert(
        "b.mjs".into(),
        "export {right, token, group} from './shared.mjs'; export {second} from './right.mjs';"
            .into(),
    );
    let (_, output) = build(&pure, &entries, false);
    let owner = |module: &str, index| {
        &output.members[&NodeId::Segment {
            module: module.into(),
            index,
        }]
    };
    assert_eq!(owner("shared.mjs", 0), &output.entries["a.mjs"]);
    assert_eq!(owner("shared.mjs", 1), &output.entries["b.mjs"]);
    assert_eq!(owner("left.mjs", 0), &output.entries["a.mjs"]);
    assert_eq!(owner("left.mjs", 1), &output.entries["a.mjs"]);
    assert_eq!(owner("right.mjs", 0), &output.entries["b.mjs"]);
    assert_eq!(owner("shared.mjs", 2), owner("shared.mjs", 3));
    assert_ne!(owner("shared.mjs", 2), &output.entries["a.mjs"]);
    assert_ne!(owner("shared.mjs", 2), &output.entries["b.mjs"]);
    for module in ["shared.mjs", "left.mjs", "right.mjs"] {
        assert!(!output.modules.contains_key(Path::new(module)));
    }
    assert_eq!(output, build(&pure, &entries, true).1);
    // Adding native phases may require separate lexical jobs. Keep the original
    // effectful program as a differential check, not a co-location requirement.
    let (_, output) = build(&source, &entries, false);
    assert_eq!(output, build(&source, &entries, true).1);
    let observer = "const a = await import('./a.mjs'); const b = await import('./b.mjs'); console.log(a.left, b.right, a.first, b.second, a.safe(), a.token === b.token, a.group === b.group, a.group(), a.group.name, a.safe.name, a.early === b.early, a.otherEarly === b.otherEarly, a.late);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        node(&baseline, Path::new("observe.mjs")).stdout
    );
}

#[test]
fn native_segment_owners_keep_mutable_accesses_native_with_pure_prefix_jobs() {
    let source = sources(&[
        (
            "a.mjs",
            "import {left, state, bump} from './shared.mjs'; import {label} from './left.mjs'; export {left, state, bump, label};",
        ),
        (
            "b.mjs",
            "import {right, state, bump} from './shared.mjs'; import {label} from './right.mjs'; export {right, state, bump, label};",
        ),
        (
            "shared.mjs",
            "export const left = () => state; export const right = () => state; export let state = 0; export function bump() { state++; } console.log('shared');",
        ),
        (
            "left.mjs",
            "export const label = () => 'left'; console.log('left effect');",
        ),
        (
            "right.mjs",
            "export const label = () => 'right'; console.log('right effect');",
        ),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true)];
    let (plan, output) = build(&source, &entries, false);
    let segment = |module: &str, index| NodeId::Segment {
        module: module.into(),
        index,
    };
    let left = &output.members[&segment("shared.mjs", 0)];
    let right = &output.members[&segment("shared.mjs", 1)];
    assert_ne!(
        left, right,
        "independent prefix demand chooses separate owners"
    );
    assert_ne!(left, &output.entries["a.mjs"]);
    assert_ne!(right, &output.entries["b.mjs"]);
    assert_ne!(left, &output.members[&segment("shared.mjs", 2)]);
    assert_eq!(
        output.members[&segment("left.mjs", 0)],
        output.members[&segment("left.mjs", 1)]
    );
    assert_eq!(
        output.members[&segment("right.mjs", 0)],
        output.members[&segment("right.mjs", 1)]
    );
    assert_eq!(
        output.members[&segment("shared.mjs", 2)],
        output.members[&segment("shared.mjs", 3)]
    );
    for (chunk, nodes) in &plan.chunks {
        for node in nodes {
            assert!(output.files.contains_key(&output.members[node]));
            assert!(output.chunk_artifacts[chunk].contains(&output.members[node]));
        }
    }
    assert_eq!(output, build(&source, &entries, true).1);
    let observer = "const a = await import('./a.mjs'); const b = await import('./b.mjs'); a.bump(); console.log(a.left(), b.right(), a.state, b.state, a.bump === b.bump, a.left.name, b.right.name, a.label.name, b.label.name, a.label(), b.label());";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let baseline = node(&baseline, Path::new("observe.mjs"));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, baseline.stdout);
}

#[test]
fn native_segment_relocation_preserves_unchanged_imports_and_default_expressions() {
    for (shared, left_index) in [
        (
            "export const left = 1; const value = 2; export {value as state}; console.log('shared', value);",
            0,
        ),
        (
            "export const left = 1; const value = 2; export {value as state}; export default 3; console.log('shared', value);",
            0,
        ),
        (
            "export const left = 1; const value = 2; const state = 4; export {value as state, state as other}; export default 3; console.log('shared', value, state);",
            0,
        ),
        (
            "const hidden = 2; export const left = () => hidden; const value = 4; export {value as state, value as hidden}; console.log('shared', hidden, value);",
            1,
        ),
    ] {
        let default = if shared.contains("export default") {
            "import value from './shared.mjs'; console.log(value);"
        } else {
            ""
        };
        let source = sources(&[
            (
                "a.mjs",
                &format!("import {{left}} from './shared.mjs'; console.log(left); {default}"),
            ),
            (
                "b.mjs",
                &format!("import {{state}} from './shared.mjs'; console.log(state); {default}"),
            ),
            ("shared.mjs", shared),
        ]);
        let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
        assert_ne!(
            output.members[&NodeId::Segment {
                module: "shared.mjs".into(),
                index: left_index,
            }],
            output.entries["a.mjs"],
            "abrupt source phases preserve their native addresses"
        );
        for entry in ["a.mjs", "b.mjs"] {
            let baseline = node(&source, Path::new(entry));
            let actual = node(&output.files, &output.entries[entry]);
            assert!(
                actual.status.success(),
                "{}",
                String::from_utf8_lossy(&actual.stderr)
            );
            assert_eq!(actual.stdout, baseline.stdout);
        }
    }
}

#[test]
fn native_segment_relocation_keeps_cross_chunk_names_distinct_from_destination_locals() {
    let source = sources(&[
        (
            "a.mjs",
            "export const x = 1; export const left = () => x; console.log('shared', x);",
        ),
        (
            "b.mjs",
            "import {x as other, left} from './a.mjs'; const x = 3; console.log(other, left(), x, left.name);",
        ),
        ("c.mjs", "import {x} from './a.mjs'; console.log(x);"),
    ]);
    let (_, output) = build(&source, &[("b.mjs", true), ("c.mjs", true)], false);
    assert_ne!(
        output.members[&NodeId::Segment {
            module: "a.mjs".into(),
            index: 1,
        }],
        output.entries["b.mjs"]
    );
    for entry in ["b.mjs", "c.mjs"] {
        let baseline = node(&source, Path::new(entry));
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
    }
}

#[test]
fn arbitrary_placement_shares_mutable_lexical_clusters_across_source_boundaries() {
    let source = sources(&[
        (
            "a.mjs",
            "import {left, state, bump} from './shared.mjs'; import {token} from './other.mjs'; export {left, state, bump, token};",
        ),
        (
            "b.mjs",
            "import {right, state, bump} from './shared.mjs'; import {token} from './other.mjs'; export {right, state, bump, token};",
        ),
        (
            "shared.mjs",
            "export let state = 0; export function bump() { state++; } export const left = () => state; export const right = () => state; console.log('shared effect');",
        ),
        ("other.mjs", "export const token = {};"),
    ]);
    let entries = [("a.mjs", true), ("b.mjs", true)];
    let (_, output) = build(&source, &entries, false);
    let segment = |module: &str, index| NodeId::Segment {
        module: module.into(),
        index,
    };
    let state = &output.members[&segment("shared.mjs", 0)];
    assert_eq!(state, &output.members[&segment("shared.mjs", 1)]);
    assert_ne!(state, &output.members[&segment("other.mjs", 0)]);
    assert_ne!(state, &output.entries["a.mjs"]);
    assert_ne!(state, &output.entries["b.mjs"]);
    assert_ne!(state, &output.members[&segment("shared.mjs", 4)]);
    insta::assert_snapshot!(
        output.files[state]
            .lines()
            .filter(|line| line.starts_with("export {"))
            .collect::<Vec<_>>()
            .join("\n"),
        @r###"
    export {state as "__gf_segment_binding_3ccc2eb2e47f503ffd17288457e128f95d9a864633f0744564bfc2a002a9a168_11"};
    export {bump as "__gf_segment_binding_3ccc2eb2e47f503ffd17288457e128f95d9a864633f0744564bfc2a002a9a168_38"};
    "###
    );
    assert_ne!(
        &output.members[&segment("shared.mjs", 2)],
        &output.entries["a.mjs"]
    );
    assert_ne!(
        &output.members[&segment("shared.mjs", 3)],
        &output.entries["b.mjs"]
    );
    assert_eq!(output, build(&source, &entries, true).1);
    let observer = "const a = await import('./a.mjs'); const b = await import('./b.mjs'); a.bump(); b.bump(); console.log(a.left(), b.right(), a.state, b.state, a.bump === b.bump, a.token === b.token, a.bump.name);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        node(&baseline, Path::new("observe.mjs")).stdout
    );
}

#[test]
fn arbitrary_placement_namespaces_do_not_retain_dead_private_chunks() {
    let source = sources(&[
        ("a.mjs", "import * as ns from './api.mjs'; console.log(Object.keys(ns).join(','), ns.live, ns.token);"),
        ("b.mjs", "import {token} from './shared.mjs'; console.log(token);"),
        ("api.mjs", "import {dead} from './dead.mjs'; const unused = () => dead; export const live = 1; export {token} from './shared.mjs';"),
        ("dead.mjs", "export const dead = () => 'DEAD_PRIVATE_BODY';"),
        ("shared.mjs", "export const token = 7;"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    let dead = NodeId::Module("dead.mjs".into());
    assert!(!output.modules.contains_key(Path::new("dead.mjs")));
    assert!(output.files.contains_key(&output.members[&dead]));
    assert!(!output
        .files
        .values()
        .any(|s| s.contains("DEAD_PRIVATE_BODY") || s.is_empty()));
    let shared = &output.members[&NodeId::Segment {
        module: "shared.mjs".into(),
        index: 0,
    }];
    assert_ne!(shared, &output.entries["a.mjs"]);
    assert_ne!(shared, &output.entries["b.mjs"]);
    for entry in ["a.mjs", "b.mjs"] {
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, node(&source, Path::new(entry)).stdout);
    }
}

#[test]
fn arbitrary_placement_preserves_initialization_snapshots_and_async_cycle_phases() {
    for items in [
        vec![
            (
                "a.mjs",
                "import {snapshot, read} from './shared.mjs'; console.log(snapshot, read());",
            ),
            (
                "b.mjs",
                "import {state, read} from './shared.mjs'; console.log(state, read());",
            ),
            (
                "shared.mjs",
                "export let state = 1; export const snapshot = state; export function read() { return state; } state = 2;",
            ),
        ],
        vec![
            (
                "a.mjs",
                "import {read, value} from './shared.mjs'; console.log(read(), value);",
            ),
            (
                "b.mjs",
                "import {read, value} from './shared.mjs'; console.log(read(), value);",
            ),
            (
                "shared.mjs",
                "console.log('before await'); await 0; export const value = 3; export function read() { return value; } console.log('after await');",
            ),
        ],
        vec![
            (
                "a.mjs",
                "import {read, value} from './shared.mjs'; console.log(read(), value);",
            ),
            (
                "b.mjs",
                "import {read, value} from './shared.mjs'; console.log(read(), value);",
            ),
            (
                "shared.mjs",
                "import './cycle.mjs'; export default function() { return 'default'; } export function read() { return 9; } export const value = 3;",
            ),
            (
                "cycle.mjs",
                "import fn from './shared.mjs'; console.log(fn(), fn.name);",
            ),
        ],
        vec![
            (
                "a.mjs",
                "import './b.mjs'; import * as ns from './c.mjs'; export function f() { return ns.value; }",
            ),
            (
                "b.mjs",
                "import {f} from './a.mjs'; try { console.log(f()); } catch (error) { console.log(error.name); }",
            ),
            ("c.mjs", "console.log('c'); export const value = 1;"),
        ],
    ] {
        let source = sources(&items);
        let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
        let graph = graph(&source, false);
        let (module, name) = if source.contains_key(Path::new("shared.mjs")) {
            ("shared.mjs", "read")
        } else {
            ("a.mjs", "f")
        };
        let file = graph.file_id(Path::new(module)).unwrap();
        let linked = graph
            .resolve_export(file, name, source_graph::Namespace::Value)
            .unwrap();
        let definition = crate::chunks::node_id(&graph, linked.declaration).unwrap();
        let evaluation = NodeId::Module(module.into());
        if module == "shared.mjs" && !output.async_modules.contains(Path::new(module)) {
            assert_ne!(
                output.members[&definition], output.members[&evaluation],
                "hoisted functions can leave their source evaluation phase"
            );
        } else {
            assert_eq!(
                output.members[&definition], output.members[&evaluation],
                "relocation cannot activate a later native dependency"
            );
        }
        for entry in ["a.mjs", "b.mjs"] {
            let baseline = node(&source, Path::new(entry));
            assert!(
                baseline.status.success(),
                "{}",
                String::from_utf8_lossy(&baseline.stderr)
            );
            let actual = node(&output.files, &output.entries[entry]);
            assert!(
                actual.status.success(),
                "{}",
                String::from_utf8_lossy(&actual.stderr)
            );
            assert_eq!(actual.stdout, baseline.stdout);
        }
    }
}

#[test]
fn arbitrary_placement_preserves_commonjs_and_namespace_activation_order() {
    let source = sources(&[
        (
            "a.mjs",
            "import {read, pure} from './shared.mjs'; console.log('a', read(), pure());",
        ),
        (
            "b.mjs",
            "import {read} from './shared.mjs'; console.log('b', read());",
        ),
        (
            "shared.mjs",
            "import './before.mjs'; import value from './dep.cjs'; import * as ns from './ns.mjs'; export function read() { return value + ns.value; } export const pure = () => 1;",
        ),
        ("before.mjs", "console.log('before');"),
        ("dep.cjs", "console.log('cjs'); module.exports = 2;"),
        (
            "ns.mjs",
            "console.log('namespace'); export const value = 3;",
        ),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    assert_ne!(
        output.members[&NodeId::Segment {
            module: "shared.mjs".into(),
            index: 4
        }],
        output.entries["a.mjs"]
    );
    for entry in ["a.mjs", "b.mjs"] {
        let baseline = node(&source, Path::new(entry));
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
    }
}

#[test]
fn arbitrary_placement_of_dynamic_commonjs_loaders_preserves_deferred_activation() {
    let source = sources(&[
        ("a.mjs", "export {load, pure} from './shared.mjs'; console.log('a');"),
        ("b.mjs", "export {load} from './shared.mjs'; console.log('b');"),
        ("shared.mjs", "export function load() { return import('./dep.cjs'); } export const pure = () => 1; console.log('shared');"),
        ("dep.cjs", "console.log('cjs-lazy'); exports.value = 6;"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    let loader = &output.members[&NodeId::Segment {
        module: "shared.mjs".into(),
        index: 0,
    }];
    assert_ne!(
        loader,
        &output.members[&NodeId::Module("shared.mjs".into())]
    );
    assert_ne!(loader, &output.entries["a.mjs"]);
    assert_ne!(loader, &output.entries["b.mjs"]);
    let observer = "const a = await import('./a.mjs'); const b = await import('./b.mjs'); console.log('before load', a.pure()); const [first, second] = await Promise.all([a.load(), b.load()]); console.log(first === second, first.value, first.default.value);";
    let mut baseline = source.clone();
    baseline.insert("observe.mjs".into(), observer.into());
    let baseline = node(&baseline, Path::new("observe.mjs"));
    let mut actual = output.files.clone();
    actual.insert("observe.mjs".into(), observation(&output, observer));
    let actual = node(&actual, Path::new("observe.mjs"));
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, baseline.stdout);
}

#[test]
fn arbitrary_placement_preserves_cyclic_connector_evaluation_barriers() {
    for dead in ["", "const unused = 17;"] {
        let source = sources(&[
            (
                "entry.mjs",
                "import {v} from './x.mjs'; console.log('entry', v);",
            ),
            (
                "x.mjs",
                &format!("import './a.mjs'; export {{v}} from './b.mjs'; {dead}"),
            ),
            ("a.mjs", "import './x.mjs'; console.log('a');"),
            ("b.mjs", "export const v = (console.log('b'), 1);"),
        ]);
        let (_, output) = build(&source, &[("entry.mjs", true)], false);
        let actual = node(&output.files, &output.entries["entry.mjs"]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, node(&source, Path::new("entry.mjs")).stdout);
        assert_eq!(actual.stdout, b"a\nb\nentry 1\n");
    }
}

#[test]
fn connector_grouping_ignores_unretained_declarations() {
    let make = |dead: &str| {
        sources(&[
            (
                "entry.mjs",
                "import {v} from './barrel.mjs'; import './side.mjs'; console.log('entry', v);",
            ),
            (
                "other.mjs",
                "import {v} from './barrel.mjs'; console.log('other', v);",
            ),
            (
                "barrel.mjs",
                &format!("export {{v}} from './dep.mjs'; {dead}"),
            ),
            ("dep.mjs", "console.log('dep'); export const v = 7;"),
            ("side.mjs", "console.log('side');"),
        ])
    };
    let (_, clean) = build(
        &make(""),
        &[("entry.mjs", true), ("other.mjs", true)],
        false,
    );
    let source = make("const unused = 23;");
    let (_, output) = build(&source, &[("entry.mjs", true), ("other.mjs", true)], false);
    assert_eq!(output.files, clean.files);
    for entry in ["entry.mjs", "other.mjs"] {
        let actual = node(&output.files, &output.entries[entry]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, node(&source, Path::new(entry)).stdout);
    }
}

#[test]
fn native_segment_relocation_preserves_tdz_effect_order_and_deferred_activation() {
    let source = sources(&[
        (
            "a.mjs",
            "import {left} from './shared.mjs'; console.log('a', left); export const load = () => import('./lazy.mjs');",
        ),
        (
            "b.mjs",
            "import {right} from './shared.mjs'; console.log('b', right);",
        ),
        (
            "shared.mjs",
            "try { console.log(left); } catch (error) { console.log(error.name); } export const left = 1; export const right = 2; console.log('shared');",
        ),
        ("lazy.mjs", "console.log('lazy'); export const value = 3;"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    let left = NodeId::Segment {
        module: "shared.mjs".into(),
        index: 1,
    };
    let right = NodeId::Segment {
        module: "shared.mjs".into(),
        index: 2,
    };
    assert_eq!(output.members[&left], output.members[&right]);
    assert_ne!(
        output.members[&left], output.entries["a.mjs"],
        "a lexical declaration cannot cross its original TDZ observation"
    );
    for (observer, expected) in [
        (
            "await import('./a.mjs'); await import('./b.mjs');",
            "ReferenceError\nshared\na 1\nb 2\n",
        ),
        (
            "const a = await import('./a.mjs'); console.log('before lazy'); console.log((await a.load()).value);",
            "ReferenceError\nshared\na 1\nbefore lazy\nlazy\n3\n",
        ),
    ] {
        let mut actual = output.files.clone();
        actual.insert("observe.mjs".into(), observation(&output, observer));
        let actual = node(&actual, Path::new("observe.mjs"));
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, expected.as_bytes());
    }
}

#[test]
fn native_barrel_redirects_preserve_exact_public_surfaces() {
    for (dependency, barrel, expected) in [
        (
            "export const x = 1; export const y = 2; export default 3;",
            "export {x} from './dep.mjs';",
            "x default,x,y true false false\n",
        ),
        (
            "export const x = 1; export const y = 2; export default 3;",
            "export {x as renamed} from './dep.mjs';",
            "renamed default,x,y false true false\n",
        ),
        (
            "export const x = 1; export const y = 2;",
            "export {x as y, y as x} from './dep.mjs';",
            "x,y x,y false false false\n",
        ),
        (
            "export const x = 1; export const y = 2; export default 3;",
            "export * from './dep.mjs';",
            "x,y default,x,y true false false\n",
        ),
        (
            "export * as x from './leaf.mjs'; export const y = 2; export default 3;",
            "export {x} from './dep.mjs';",
            "x default,x,y true false false\n",
        ),
    ] {
        let source = sources(&[
            ("entry.mjs", "export * from './barrel.mjs';"),
            ("barrel.mjs", barrel),
            ("dep.mjs", dependency),
            ("leaf.mjs", "export const value = 1;"),
        ]);
        let (_, output) = build(&source, &[("entry.mjs", true), ("dep.mjs", true)], false);
        assert_eq!(output.backend, Backend::Native);
        let check = |entry: &Path, dep: &Path| {
            format!(
            "import * as entry from {}; import * as dep from {}; console.log(Object.keys(entry).join(','), Object.keys(dep).join(','), entry.x === dep.x, entry.renamed === dep.x, 'default' in entry);",
            quote(&format!("./{}", entry.display())), quote(&format!("./{}", dep.display()))
        )
        };
        let mut native = source.clone();
        native.insert(
            "check.mjs".into(),
            check(Path::new("entry.mjs"), Path::new("dep.mjs")),
        );
        let baseline = node(&native, Path::new("check.mjs"));
        let mut files = output.files.clone();
        files.insert(
            "check.mjs".into(),
            check(&output.entries["entry.mjs"], &output.entries["dep.mjs"]),
        );
        let actual = node(&files, Path::new("check.mjs"));
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
        assert_eq!(actual.stdout, expected.as_bytes());
    }
}

#[test]
fn native_chunk_namespace_ambiguity_survives_other_entry_demands() {
    let source = sources(&[
        (
            "barrel.mjs",
            "export * from './left.mjs'; export * from './right.mjs';",
        ),
        ("left.mjs", "export const conflict = 1;"),
        (
            "right.mjs",
            "export const conflict = 2; export const kept = 3;",
        ),
    ]);
    let (_, output) = build(&source, &[("barrel.mjs", true), ("left.mjs", true)], false);
    assert_eq!(output.backend, Backend::Native);
    let mut files = output.files.clone();
    files.insert(
        "check.mjs".into(),
        format!(
            "import * as ns from {}; console.log(Object.keys(ns).join(','), ns.conflict, ns.kept);",
            quote(&format!("./{}", output.entries["barrel.mjs"].display()))
        ),
    );
    let actual = node(&files, Path::new("check.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"kept undefined 3\n");
}

#[test]
fn native_chunks_share_precise_namespace_values_across_public_entries() {
    let source = sources(&[
        (
            "leaf.mjs",
            "export let value = 1; export function bump() { value++; }",
        ),
        ("a.mjs", "export * as ns from './leaf.mjs';"),
        ("route.mjs", "export {ns as renamed} from './a.mjs';"),
        ("b.mjs", "export {renamed as alias} from './route.mjs';"),
    ]);
    let (_, output) = build(&source, &[("a.mjs", true), ("b.mjs", true)], false);
    assert_eq!(output.backend, Backend::Native);
    let mut files = output.files.clone();
    files.insert("check.mjs".into(), format!(
        "import {{ns}} from {}; import {{alias}} from {}; ns.bump(); const d = Object.getOwnPropertyDescriptor(alias, 'value'); console.log(ns === alias, Object.keys(ns).join(','), alias['value'], d.value, d.writable, d.configurable, Object.getPrototypeOf(ns) === null, Object.isExtensible(ns));",
        quote(&format!("./{}", output.entries["a.mjs"].display())),
        quote(&format!("./{}", output.entries["b.mjs"].display()))
    ));
    let actual = node(&files, Path::new("check.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"true bump,value 2 2 true false true false\n"
    );
}

#[test]
fn native_cycle_instantiation_distinguishes_functions_var_and_lexical_tdz() {
    let output = compare(&sources(&[
        ("entry.mjs", "import {read, bump, value} from './a.mjs'; console.log(read(), value); bump(); console.log(read(), value);"),
        ("a.mjs", "import {initial} from './b.mjs'; export var value = initial + 1; export function read() { return value; } export function bump() { value++; }"),
        ("b.mjs", "import {read, value} from './a.mjs'; console.log(typeof read, value, read()); export var initial = 4;"),
    ]), "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    assert!(!output
        .files
        .values()
        .any(|s| s.contains("export const read =")));
    compare(
        &sources(&[
            ("entry.mjs", "import './a.mjs';"),
            ("a.mjs", "import './b.mjs'; export let value = 1;"),
            (
                "b.mjs",
                "import {value} from './a.mjs'; console.log(value);",
            ),
        ]),
        "entry.mjs",
    );
}

#[test]
fn namespace_reexports_live_aliases_and_default_are_not_snapshots() {
    compare(&sources(&[
        ("entry.mjs", "import * as ns from './barrel.mjs'; import {renamed} from './barrel.mjs'; console.log(Object.keys(ns).join(','), ns.default, renamed); ns.bump(); console.log(ns.value, ns.renamed, renamed, ns.box.value);"),
        ("barrel.mjs", "export * from './leaf.mjs'; export {value as renamed, default} from './leaf.mjs'; export * as box from './leaf.mjs';"),
        ("leaf.mjs", "export let value = 1; export function bump() { value++; } export default 42; const unused = 'must disappear';"),
    ]), "entry.mjs");
    compare(&sources(&[
        ("entry.mjs", "import * as ns from './barrel.mjs'; console.log(Object.keys(ns).join(','), ns.value);"),
        ("barrel.mjs", "export * from './a.mjs'; export * from './b.mjs'; export {value} from './a.mjs';"),
        ("a.mjs", "export const value = 1; export const conflict = 2; export default 3;"),
        ("b.mjs", "export const value = 4; export const conflict = 5;"),
    ]), "entry.mjs");
}

#[test]
fn drops_unused_import_bindings_and_reexports_but_not_static_effects() {
    let output = compare(&sources(&[
        ("entry.mjs", "import {kept} from './barrel.mjs'; import {unused} from './barrel.mjs'; console.log(kept);"),
        ("barrel.mjs", "export {kept} from './leaf.mjs'; export {unused} from './leaf.mjs'; console.log('barrel');"),
        ("leaf.mjs", "export const kept = 1; export const unused = 'must disappear'; console.log('leaf');"),
    ]), "entry.mjs");
    assert!(!output.files.values().any(|s| s.contains("must disappear")));
}

#[test]
fn logical_root_sets_project_to_actual_native_body_owners() {
    let sources = sources(&[
        ("a.mjs", "export {a} from './common.mjs';"),
        ("b.mjs", "export {b} from './common.mjs';"),
        ("c.mjs", "export {a as ca, b as cb} from './common.mjs';"),
        ("common.mjs", "export let a = {}; export let b = {}; function unused() { return 'must disappear'; }"),
        ("observe.mjs", "import {a} from './a.mjs'; import {b} from './b.mjs'; import {ca, cb} from './c.mjs'; console.log(a === b, ca === cb, a === ca, b === cb);"),
    ]);
    let entries = [
        ("a.mjs", true),
        ("b.mjs", true),
        ("c.mjs", true),
        ("observe.mjs", false),
    ];
    let (plan, output) = build(&sources, &entries, false);
    assert!(plan.chunks.len() >= 3);
    let module = PathBuf::from("common.mjs");
    let first = NodeId::Segment {
        module: module.clone(),
        index: 0,
    };
    let second = NodeId::Segment { module, index: 1 };
    assert_ne!(plan.chunk_for(&first), plan.chunk_for(&second));
    assert_eq!(output.backend, super::Backend::Native);
    assert_ne!(output.members[&first], output.members[&second]);
    assert!(output.members[&first]
        .extension()
        .is_some_and(|e| e == "mjs"));
    for (chunk, members) in &plan.chunks {
        for member in members {
            assert!(output.chunk_artifacts[chunk].contains(&output.members[member]));
            assert_eq!(plan.chunk_for(member).as_ref(), Some(chunk));
        }
    }
    assert!(!output.files.values().any(|s| s.contains("must disappear")));
    let baseline = node(&sources, Path::new("observe.mjs"));
    let bundled = node(&output.files, &output.entries["observe.mjs"]);
    assert!(
        bundled.status.success(),
        "{}",
        String::from_utf8_lossy(&bundled.stderr)
    );
    assert_eq!(bundled.stdout, baseline.stdout);
    let (_, reversed) = build(&sources, &entries, true);
    assert_eq!(output.files, reversed.files);
    assert_eq!(output.entries, reversed.entries);
    assert_eq!(output.members, reversed.members);
}

#[test]
fn statement_sentinels_have_truthful_owners_and_async_load_closure() {
    let source = sources(&[
        ("a.mjs", "export {a} from './parts.mjs';"),
        ("b.mjs", "export {b} from './parts.mjs';"),
        ("main.mjs", "export const later = () => import('./parts.mjs'); console.log('ready');"),
        ("parts.mjs", "export const a = 'physical-sentinel-A'; export const b = 'physical-sentinel-B'; export const c = 'physical-sentinel-C'; console.log('parts effect');"),
    ]);
    let (plan, mut output) = build(
        &source,
        &[("a.mjs", true), ("b.mjs", true), ("main.mjs", true)],
        false,
    );
    let paths: Vec<_> = (0..3)
        .map(|index| {
            output.members[&NodeId::Segment {
                module: "parts.mjs".into(),
                index,
            }]
                .clone()
        })
        .collect();
    for (index, sentinel) in [
        "physical-sentinel-A",
        "physical-sentinel-B",
        "physical-sentinel-C",
    ]
    .iter()
    .enumerate()
    {
        let owners: Vec<_> = output
            .files
            .iter()
            .filter(|(_, body)| body.contains(sentinel))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(owners, vec![&paths[index]]);
    }
    for (chunk, nodes) in &plan.chunks {
        for node in nodes {
            assert!(output.chunk_artifacts[chunk].contains(&output.members[node]));
        }
    }
    fn closure(output: &Artifacts, entry: &Path) -> BTreeSet<PathBuf> {
        use oxc_ast::ast::*;
        use oxc_ast_visit::{walk, Visit};
        #[derive(Default)]
        struct Edges(Vec<String>);
        impl<'a> Visit<'a> for Edges {
            fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
                self.0.push(import.source.value.to_string());
            }
            fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
                if let Some(source) = &export.source {
                    self.0.push(source.value.to_string());
                }
            }
            fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
                self.0.push(export.source.value.to_string());
            }
            fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
                if matches!(&call.callee,Expression::Identifier(i) if i.name == "require") {
                    if let Some(Argument::StringLiteral(source)) = call.arguments.first() {
                        self.0.push(source.value.to_string());
                    }
                }
                walk::walk_call_expression(self, call);
            }
            fn visit_import_expression(&mut self, _: &ImportExpression<'a>) {}
            fn visit_function(&mut self, _: &Function<'a>, _: oxc_semantic::ScopeFlags) {}
            fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
        }
        let mut found = BTreeSet::new();
        let mut todo = vec![entry.to_owned()];
        while let Some(path) = todo.pop() {
            if !found.insert(path.clone()) {
                continue;
            }
            let allocator = oxc_allocator::Allocator::default();
            let parsed = oxc_parser::Parser::new(
                &allocator,
                &output.files[&path],
                oxc_span::SourceType::mjs(),
            )
            .parse();
            let mut edges = Edges::default();
            edges.visit_program(&parsed.program);
            todo.extend(
                edges
                    .0
                    .iter()
                    .filter(|s| s.starts_with('.'))
                    .map(|s| normalize(&path.parent().unwrap().join(s))),
            );
        }
        found
    }
    let initial = closure(&output, &output.entries["main.mjs"]);
    assert!(paths.iter().all(|p| !initial.contains(p)));
    let a_load = closure(&output, &output.entries["a.mjs"]);
    assert!(a_load.contains(&paths[0]));
    let observer = observation(&output,"const main = await import('./main.mjs'); console.log('invoke'); const [a,b] = await Promise.all([main.later(),main.later()]); console.log(a === b,a.a,a.b,a.c);");
    output.files.insert("observe.mjs".into(), observer);
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout,b"ready\ninvoke\nparts effect\ntrue physical-sentinel-A physical-sentinel-B physical-sentinel-C\n");
}

#[test]
fn native_binding_operations_preserve_keys_names_strict_this_and_const_errors() {
    compare(&sources(&[
        ("entry.mjs","import value, {a, next, C, read} from './state.mjs'; console.log(value.name, C.self === C, C.name, read() === undefined, a.name); next(); console.log(a.name);"),
        ("state.mjs","export let {a = () => 1} = {}; export function next() { ({a = () => 2} = {}); } export class C { static self = this; } export function read() { return this; } export default () => 3;"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.mjs","import {test} from './state.mjs'; console.log(test());"),
        ("state.mjs","const fixed = 1; export function test() { try { fixed++; } catch(e) { return e.name; } }"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.mjs","console.log(typeof arguments, typeof __gf_runtime); try { arguments; } catch(e) { console.log(e.name); } const fn = () => typeof arguments; console.log(fn()); class C { [String(this)]() { return 1; } static value = this; instance = this; } const value = new C(); console.log(value.undefined(),C.value === C,value.instance === value); export const __gf_setter_0 = 7; console.log(__gf_setter_0);"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.mjs","import './cycle.mjs'; import * as self from './entry.mjs'; export var value = 1; export function bump() { value++; } console.log(self.value);"),
        ("cycle.mjs","import * as ns from './entry.mjs'; console.log(typeof ns.bump,ns.value); ns.bump(); console.log(ns.value);"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.mjs","export class C {static own=C; static make(){return new C();}} const original = C; C = 1; console.log(original.own === original,original.make() instanceof original);"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.mjs","const Ref = ReferenceError, Type = TypeError; globalThis.ReferenceError = Error; globalThis.TypeError = Error; try {value;} catch(e) {console.log(e instanceof Ref);} let value = 1; const fixed = 2; try {fixed++;} catch(e) {console.log(e instanceof Type);}"),
    ]),"entry.mjs");
    compare(&sources(&[
        ("entry.cjs","console.log(typeof __gf_runtime,typeof __gf_runtime$); try {__gf_runtime;} catch(e) {console.log(e.name);} globalThis.__gf_runtime = 3; console.log(__gf_runtime);"),
    ]),"entry.cjs");
}

#[test]
fn native_for_await_preserves_cached_next_and_close_completion() {
    for body in [
        "let n=0; const iterator = { [Symbol.asyncIterator]() {return this;}, next() {this.next = () => {throw Error('replaced next');}; return Promise.resolve({value:++n,done:n>2});}, get return() {console.log('return getter'); return () => {console.log('close'); return Promise.resolve({done:true});};} }; for await (const value of iterator) {console.log(value); if(value===2) break;}",
        "const error = Error('original'); const iterator = {[Symbol.asyncIterator]() {return this;}, next() {return Promise.resolve({value:1,done:false});}, return() {console.log('close'); return Promise.reject(Error('close failure'));}}; try {for await(const value of iterator) {throw error;}} catch(e) {console.log(e === error);}",
        "const iterator = {[Symbol.iterator]() {return this;}, next() {return {value:Promise.resolve(1),done:false};}, return() {console.log('sync close'); return {done:true,value:Promise.resolve(2)};}}; for await(const value of iterator) {console.log(value); break;} console.log('after');",
        "let count=0; const iterator = {[Symbol.asyncIterator]() {return this;}, next() {return Promise.resolve({value:++count,done:count>3});}, return() {console.log('close');return Promise.resolve({done:true});}}; outer: inner: for await(const value of iterator) {if(value===1) continue outer; console.log(value); break inner;} console.log('after');",
    ] {
        let output = compare(&sources(&[("entry.mjs",body)]),"entry.mjs");
        assert_eq!(output.backend, Backend::Native);
    }
}

#[test]
fn namespace_forward_tdz_and_same_statement_initialization_are_native_observable() {
    compare(&sources(&[
        ("entry.mjs","import * as self from './entry.mjs'; try { console.log(self.value); } catch(e) { console.log(e.name); } export let value = 1; console.log(self.value);"),
    ]),"entry.mjs");
    compare(
        &sources(&[(
            "entry.mjs",
            "import * as self from './entry.mjs'; export let [a,b = self.a] = [1]; console.log(b);",
        )]),
        "entry.mjs",
    );
}

#[test]
fn deferred_namespace_binding_preserves_late_identity_and_initial_observations() {
    for owner in [
        "import * as ns from './target.mjs'; export const value = 1; export function read() { return ns.extra; }",
        "import value, * as unused from './target.mjs'; export const valueForEntry = 1; export {valueForEntry as value}; export function read() { return value; }",
        "export * as ns from './target.mjs'; export const value = 1; export function read() { return 'namespace-deferred-sentinel'; }",
    ] {
    let source = sources(&[
        ("entry.mjs","import {value} from './owner.mjs'; console.log(value); export const later = () => import('./owner.mjs'); export const target = () => import('./target.mjs');"),
        ("owner.mjs",owner),
        ("target.mjs","export const extra = 'namespace-deferred-sentinel'; export default 'namespace-deferred-sentinel';"),
    ]);
    let (plan, mut output) = build(&source, &[("entry.mjs", true),("target.mjs",true)], false);
    let target = NodeId::Segment {
        module: "target.mjs".into(),
        index: 0,
    };
    assert!(plan.node_roots[&target]
        .iter()
        .all(|r| !matches!(r, RootId::Entry(name) if name == "entry.mjs")));
    let observer = observation(&output,"const entry = await import('./entry.mjs'); const owner = await entry.later(); const target = await entry.target(); console.log(target === await import('./target.mjs'), owner.ns ? owner.ns === target : true,owner.read());");
    output.files.insert("observe.mjs".into(),observer);
    let complete = node(&output.files,Path::new("observe.mjs"));
    assert!(complete.status.success(),"{}",String::from_utf8_lossy(&complete.stderr));
    assert_eq!(complete.stdout,b"1\ntrue true namespace-deferred-sentinel\n");
    let directory = TmpDir::new();
    for (path, body) in &output.files {
        let file = directory.root().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    // Pure statically imported namespace values may be available before demand.
    let actual = Command::new("node")
        .arg(directory.root().join(&output.entries["entry.mjs"]))
        .current_dir(directory.root())
        .output()
        .unwrap();
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"1\n");
    }
}

#[test]
fn commonjs_snapshot_subscription_does_not_resnapshot_on_later_namespace_load() {
    compare(
        &sources(&[
            (
                "entry.mjs",
                "export {read} from './loader.cjs'; console.log('ready');",
            ),
            (
                "loader.cjs",
                "exports.read = () => require('./bridge.mjs').value;",
            ),
            (
                "bridge.mjs",
                "import {value} from './dep.cjs'; export {value}; console.log('bridge');",
            ),
            (
                "dep.cjs",
                "console.log('deferred factory'); exports.value = 2;",
            ),
        ]),
        "entry.mjs",
    );
    compare(&sources(&[
        ("entry.mjs","export {read} from './loader.cjs'; console.log('ready');"),
        ("loader.cjs","exports.read = () => require('./bridge.mjs').value;"),
        ("bridge.mjs","import * as ns from './target.mjs'; export const value = ns.value; console.log('bridge');"),
        ("target.mjs","console.log('deferred target'); export const value = 2;"),
    ]),"entry.mjs");
    let source = sources(&[
        ("entry.mjs","import {value} from './owner.mjs'; console.log(value); export const later = () => import('./owner.mjs');"),
        ("owner.mjs","import value from './dep.cjs'; import {late} from './dep.cjs'; export const valueForEntry = value.ready; export {valueForEntry as value}; export {late}; export function read() {return late;}"),
        ("dep.cjs","module.exports = {ready:1,get late(){console.log('late getter');return 2;}};"),
    ]);
    let (_, mut output) = build(&source, &[("entry.mjs", true)], false);
    let initial = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    assert_eq!(initial.stdout, b"late getter\n1\n");
    let observer = observation(&output,"const e = await import('./entry.mjs'); const owner = await e.later(); console.log(owner.late,owner.read());");
    output.files.insert("observe.mjs".into(), observer);
    let complete = node(&output.files, Path::new("observe.mjs"));
    assert!(
        complete.status.success(),
        "{}",
        String::from_utf8_lossy(&complete.stderr)
    );
    assert_eq!(complete.stdout, b"late getter\n1\n2 2\n");
}

#[test]
fn commonjs_public_artifact_paths_are_executable_without_evaluating_colocated_entries() {
    let source = sources(&[(
        "entry.cjs",
        "module.exports = {value:1, replace() {module.exports = {value:2};}};",
    )]);
    let (_, mut output) = build(&source, &[("entry.cjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    output.files.insert(
        "observe.cjs".into(),
        format!(
        "const value = require({0}); value.replace(); console.log(value.value,require({0}).value);",
        quote(&format!("./{}", output.entries["entry.cjs"].display()))
    ),
    );
    let actual = node(&output.files, Path::new("observe.cjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"1 2\n");
    let source = sources(&[
        (
            "entry.mjs",
            "import {x} from './dep.cjs'; console.log('entry',x);",
        ),
        ("dep.cjs", "exports.x = 2;"),
    ]);
    let (_, mut output) = build(&source, &[("entry.mjs", true)], false);
    let proxy = &output.cjs_proxies[Path::new("dep.cjs")].0;
    output.files.insert("observe.mjs".into(),format!("const ns = await import({}); console.log(Object.keys(ns).join(','),ns.x,ns.default.x === ns.x);",quote(&format!("./{}",proxy.display()))));
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"default,x 2 true\n");
    let source = sources(&[
        (
            "entry.cjs",
            "console.log('entry'); exports.read = () => require('./dep.cjs');",
        ),
        ("dep.cjs", "module.exports = {value:2};"),
    ]);
    let (_, mut output) = build(&source, &[("entry.cjs", false), ("dep.cjs", false)], false);
    let module = &output.modules[Path::new("dep.cjs")];
    assert!(output.files.contains_key(module));
    output.files.insert(
        "observe.mjs".into(),
        format!(
            "console.log((await import({})).default.value);",
            quote(&format!("./{}", module.display()))
        ),
    );
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"2\n");
}

#[test]
fn same_logical_chunk_does_not_flatten_static_evaluation_order() {
    compare(
        &sources(&[
            (
                "entry.mjs",
                "import './a.mjs'; import './b.mjs'; console.log('entry');",
            ),
            ("a.mjs", "import './shared.mjs'; console.log('a');"),
            ("b.mjs", "import './shared.mjs'; console.log('b');"),
            ("shared.mjs", "console.log('shared');"),
        ]),
        "entry.mjs",
    );
    compare(
        &sources(&[
            ("entry.mjs", "import * as ns from './barrel.mjs'; console.log(Object.keys(ns).join(','), ns.x); ns.bump(); console.log(ns.x);"),
            ("barrel.mjs", "export * from './a.mjs'; export * from './b.mjs';"),
            ("a.mjs", "export {x, bump} from './leaf.mjs'; console.log('a');"),
            ("b.mjs", "export {x, bump} from './leaf.mjs'; console.log('b');"),
            ("leaf.mjs", "export let x = 1; export function bump() { x++; }"),
        ]),
        "entry.mjs",
    );
}

#[test]
fn dynamic_sites_share_native_namespaces_and_preserve_nested_activation_and_timing() {
    let output = compare(&sources(&[
        ("entry.mjs", "import * as sync from './shared.mjs'; console.log('before'); const first = import('./shared.mjs'); const second = import('./shared.mjs'); console.log(first !== second); Promise.resolve().then(() => console.log('microtask')); Promise.all([first, second, import('./outer.mjs')]).then(async ([a,b,outer]) => { console.log(a === b, a === sync, a.count); console.log((await outer.later()).value); console.log((await import('./outer.mjs')) === outer); }); function dead() { return import('./dead.mjs'); }"),
        ("shared.mjs", "console.log('shared'); export const count = (globalThis.count = (globalThis.count || 0) + 1);"),
        ("outer.mjs", "console.log('outer'); export function later() { return import('./inner.mjs'); }"),
        ("inner.mjs", "console.log('inner'); export const value = 7;"),
        ("dead.mjs", "console.log('must not execute');"),
    ]), "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    assert_eq!(output.async_facades.len(), 5);
    assert!(!output.modules.contains_key(Path::new("dead.mjs")));
    assert!(output
        .files
        .keys()
        .all(|p| p.extension().is_some_and(|e| e == "mjs" || e == "cjs")));
}

#[test]
fn dynamic_commonjs_uses_the_same_conservative_namespace_as_static_and_entry_loads() {
    let source = sources(&[
        ("entry.mjs", "import * as sync from './dep.cjs'; export async function check() { const [a,b] = await Promise.all([import('./dep.cjs'), import('./dep.cjs')]); console.log(a === b, a === sync, a.default === sync.default, a.computedName); a.bump(); console.log(a.computedName); return a; }"),
        ("dep.cjs", "console.log('factory once'); let value = 1; module.exports = {['computed' + 'Name']: value, bump() { module.exports.computedName = ++value; }};"),
    ]);
    let (_, mut output) = build(&source, &[("entry.mjs", true), ("dep.cjs", false)], false);
    let observer = observation(&output, "const entry = await import('./entry.mjs'); const ns = await entry.check(); console.log(ns === await import('./dep.cjs'));");
    output.files.insert("observe.mjs".into(), observer);
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    insta::assert_snapshot!(String::from_utf8(actual.stdout).unwrap(), @"
    factory once
    true true true 1
    2
    true
    ");
}

#[test]
fn dynamic_failures_are_asynchronous_shared_and_cached_by_native_esm() {
    compare(&sources(&[
        ("entry.mjs", "console.log('before'); const a = import('./fail.mjs'); console.log('after'); Promise.allSettled([a, import('./fail.mjs')]).then(async ([a,b]) => { console.log(a.status, a.reason === b.reason, a.reason.message); try { await import('./fail.mjs'); } catch(e) { console.log(e === a.reason, globalThis.attempt); } });"),
        ("fail.mjs", "globalThis.attempt = (globalThis.attempt || 0) + 1; throw Error('execute failure');"),
    ]), "entry.mjs");
    compare(&sources(&[
        ("entry.cjs", "console.log('sync'); module.exports = () => import('./dep.mjs'); Promise.all([module.exports(), module.exports()]).then(([a,b]) => console.log(a === b, a.value));"),
        ("dep.mjs", "console.log('esm once'); export const value = 9;"),
    ]), "entry.cjs");
}

#[test]
fn dynamic_native_file_load_and_parse_failures_reject_without_eager_execution() {
    let driver = |restore: &BTreeMap<PathBuf, String>| {
        format!(
        "import {{writeFileSync}} from 'node:fs'; const entry = await import('./entry.mjs'); console.log('entry ready'); const results = await Promise.allSettled([entry.later(), entry.later()]); console.log(results.every(r => r.status === 'rejected'), results[0].reason.name); const restore = {}; for (const [path,source] of Object.entries(restore)) writeFileSync(new URL(path, import.meta.url), source); try {{ await entry.later(); console.log('retry succeeded'); }} catch(e) {{ console.log('rejected', e.name); }}",
        serde_json::to_string(restore).unwrap()
    )
    };
    let source = sources(&[
        (
            "entry.mjs",
            "export function later() { return import('./dep.mjs'); }",
        ),
        (
            "dep.mjs",
            "console.log('unexpected evaluation'); export const value = 1;",
        ),
    ]);
    for malformed in [false, true] {
        let mut native = source.clone();
        let original = native.remove(Path::new("dep.mjs")).unwrap();
        if malformed {
            native.insert("dep.mjs".into(), "invalid syntax !!!".into());
        }
        native.insert(
            "observe.mjs".into(),
            driver(&BTreeMap::from([("dep.mjs".into(), original)])),
        );
        let baseline = node(&native, Path::new("observe.mjs"));
        assert!(
            baseline.status.success(),
            "{}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        let (_, mut output) = build(&source, &[("entry.mjs", true)], false);
        let path = output.modules[Path::new("dep.mjs")].clone();
        let mut restore = BTreeMap::new();
        let original = output.files.remove(&path).unwrap();
        restore.insert(path.clone(), original);
        if malformed {
            output.files.insert(path, "invalid syntax !!!".into());
        }
        let observer = observation(&output, &driver(&restore));
        output.files.insert("observe.mjs".into(), observer);
        let actual = node(&output.files, Path::new("observe.mjs"));
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, baseline.stdout);
        assert!(actual.stdout.starts_with(if malformed {
            b"entry ready\ntrue SyntaxError\n".as_slice()
        } else {
            b"entry ready\ntrue Error\n".as_slice()
        }));
    }
}

#[test]
fn tla_static_dependency_chains_diamonds_and_for_await_use_native_order() {
    let output = compare(&sources(&[
        ("entry.mjs", "console.log('entry'); Promise.all([import('./a.mjs'), import('./a.mjs'), import('./b.mjs')]).then(([a, again, b]) => console.log(a === again, a.value, b.value, globalThis.count));"),
        ("a.mjs", "import {value} from './barrel.mjs'; console.log('a', value); export {value};"),
        ("b.mjs", "import {value} from './leaf.mjs'; await 0; console.log('b', value); export {value};"),
        ("barrel.mjs", "export {value} from './leaf.mjs'; console.log('barrel');"),
        ("leaf.mjs", "console.log('leaf before'); globalThis.count = (globalThis.count || 0) + 1; export let value = await Promise.resolve(7); for await (const x of [1,2]) { console.log('iterate', x); } console.log('leaf after');"),
    ]), "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    assert_eq!(
        output.async_modules,
        BTreeSet::from([
            "a.mjs".into(),
            "b.mjs".into(),
            "barrel.mjs".into(),
            "leaf.mjs".into()
        ])
    );
}

#[test]
fn tla_cycles_preserve_hoisting_suspension_order_and_tdz_errors() {
    let output = compare(&sources(&[
        ("entry.mjs", "import {value} from './a.mjs'; console.log('entry', value);"),
        ("a.mjs", "import {other} from './b.mjs'; console.log('a before', other); export function read() { return 'hoisted'; } export const value = await Promise.resolve(other + 1); console.log('a after');"),
        ("b.mjs", "import {read} from './a.mjs'; console.log('b before', read()); await 0; console.log('b after'); export const other = 4;"),
    ]), "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    assert_eq!(output.async_modules.len(), 3);
    compare(
        &sources(&[
            ("entry.mjs", "import './a.mjs';"),
            (
                "a.mjs",
                "import './b.mjs'; export const value = await Promise.resolve(1);",
            ),
            (
                "b.mjs",
                "import {value} from './a.mjs'; await 0; console.log(value);",
            ),
        ]),
        "entry.mjs",
    );
}

#[test]
fn tla_rejection_is_shared_and_propagates_through_static_cycles() {
    let output = compare(&sources(&[
        ("entry.mjs", "Promise.allSettled([import('./a.mjs'), import('./b.mjs'), import('./a.mjs')]).then(async results => { console.log(results.every(r => r.status === 'rejected'), results[0].reason === results[1].reason, results[0].reason === results[2].reason, results[0].reason.message, globalThis.attempt); try { await import('./a.mjs'); } catch(e) { console.log(e === results[0].reason); } });"),
        ("a.mjs", "import './b.mjs'; console.log('unreachable a'); export const value = 1;"),
        ("b.mjs", "import './a.mjs'; globalThis.attempt = (globalThis.attempt || 0) + 1; await Promise.reject(Error('async failure')); console.log('unreachable b');"),
    ]), "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
}

#[test]
fn commonjs_path_parameters_preserve_redeclarations_and_factory_local_writes() {
    compare(
        &sources(&[
            (
                "entry.mjs",
                "import first from './first.cjs'; import second from './second.cjs'; \
                 console.log(first.types, second.types, first.nested()); \
                 first.change(); console.log(first.current(), second.unchanged());",
            ),
            (
                "first.cjs",
                "var __filename; if (false) { var __dirname; } \
                 exports.types = typeof __filename + ' ' + typeof __dirname; \
                 exports.nested = () => { const __filename = 'nested'; return __filename; }; \
                 exports.change = () => { __filename = 'changed'; __dirname = 'changed'; }; \
                 exports.current = () => __filename + ' ' + __dirname;",
            ),
            (
                "second.cjs",
                "var __filename; var __dirname; \
                 const filename = __filename, dirname = __dirname; \
                 exports.types = typeof __filename + ' ' + typeof __dirname; \
                 exports.unchanged = () => __filename === filename && __dirname === dirname;",
            ),
        ]),
        "entry.mjs",
    );
    compare(
        &sources(&[(
            "entry.cjs",
            "'use strict'; var __filename; var __dirname; \
             console.log(typeof __filename, typeof __dirname); \
             function shadow(__filename, __dirname) { return __filename + __dirname; } \
             console.log(shadow('nested', ' local'));",
        )]),
        "entry.cjs",
    );
    compare(
        &sources(&[(
            "entry.cjs",
            "function __filename() { return 'filename'; } \
             function __dirname() { return 'dirname'; } \
             console.log(__filename(), __dirname());",
        )]),
        "entry.cjs",
    );
}

#[test]
fn tla_dynamic_commonjs_keeps_deferred_factory_in_a_separate_native_file() {
    let source = sources(&[
        ("entry.mjs", "console.log('before'); const ns = await import('./a.cjs'); console.log('after', ns.default.value);"),
        ("a.cjs", "console.log('factory a'); exports.value = 1; exports.later = () => require('./b.cjs');"),
        ("b.cjs", "console.log('must not execute'); module.exports = 2;"),
    ]);
    let output = compare(&source, "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    let a = NodeId::Segment {
        module: "a.cjs".into(),
        index: 0,
    };
    let b = NodeId::Segment {
        module: "b.cjs".into(),
        index: 0,
    };
    // Deferred synchronous require supplies code under the same async root,
    // but loading their shared chunk must not invoke the deferred factory.
    assert_eq!(output.members[&a], output.members[&b]);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert_eq!(actual.stdout, b"before\nfactory a\nafter 1\n");
}

#[test]
fn tla_dynamic_commonjs_proxy_failure_is_cached_but_native_require_can_retry() {
    let source = sources(&[
        ("entry.mjs", "import retry from './retry.cjs'; const first = await import('./fail.cjs').catch(e => e); const second = await import('./fail.cjs').catch(e => e); console.log(first === second, first.message, globalThis.attempt); console.log(retry());"),
        ("retry.cjs", "module.exports = () => { try { require('./fail.cjs'); } catch(e) { return e.message; } };"),
        ("fail.cjs", "globalThis.attempt = (globalThis.attempt || 0) + 1; throw Error('attempt ' + globalThis.attempt);"),
    ]);
    let output = compare(&source, "entry.mjs");
    assert_eq!(output.backend, Backend::Native);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert_eq!(actual.stdout, b"true attempt 1 1\nattempt 2\n");
}

#[test]
fn tla_concurrent_entries_and_dynamic_sites_evaluate_once_and_share_namespace() {
    let source = sources(&[
        ("a.mjs", "export * as shared from './leaf.mjs';"),
        ("b.mjs", "export * as shared from './leaf.mjs';"),
        ("lazy.mjs", "export function later() { return import('./leaf.mjs'); }"),
        ("leaf.mjs", "globalThis.count = (globalThis.count || 0) + 1; console.log('before await'); await Promise.resolve(); console.log('after await'); export const count = globalThis.count;"),
    ]);
    let (_, mut output) = build(
        &source,
        &[("a.mjs", true), ("b.mjs", true), ("lazy.mjs", true)],
        false,
    );
    let observer = observation(&output, "const [a,b,lazy] = await Promise.all([import('./a.mjs'),import('./b.mjs'),import('./lazy.mjs')]); const [x,y] = await Promise.all([lazy.later(),lazy.later()]); console.log(a.shared === b.shared, a.shared === x, x === y, x.count);");
    assert_eq!(output.backend, Backend::Native);
    output.files.insert("observe.mjs".into(), observer);
    let actual = node(&output.files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"before await\nafter await\ntrue true true 1\n"
    );
}

#[test]
fn tla_awaited_dynamic_cycle_preserves_native_unsettled_evaluation_exit() {
    let source = sources(&[
        (
            "entry.mjs",
            "console.log('entry'); await import('./a.mjs'); console.log('unreachable');",
        ),
        ("a.mjs", "console.log('a'); await import('./entry.mjs');"),
    ]);
    let (_, output) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(output.backend, Backend::Native);
    let baseline = node(&source, Path::new("entry.mjs"));
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert_eq!(baseline.status.code(), Some(13));
    assert_eq!(actual.status.code(), baseline.status.code());
    assert_eq!(actual.stdout, baseline.stdout);
    assert!(String::from_utf8_lossy(&actual.stderr).contains("unsettled top-level await"));
}

#[test]
fn cached_async_facts_flow_through_graph_retention_chunks_and_native_output() {
    let source = sources(&[
        ("entry.mjs", "const [a,b] = await Promise.all([import('./a.mjs'),import('./a.mjs')]); console.log(a === b, a.value);"),
        ("a.mjs", "export {value} from './leaf.mjs';"),
        ("leaf.mjs", "console.log('async once'); export const value = await Promise.resolve(3);"),
    ]);
    let inputs = source
        .iter()
        .map(|(path, text)| {
            let parsed =
                parse_module(&logger::StdioLogger::new(), text, path, ModuleFormat::Esm).unwrap();
            let record = segment_cache::CacheRecord::from_parsed(text, &parsed).unwrap();
            let bytes = record.encode(text).unwrap();
            let decoded = segment_cache::CacheRecord::decode(&bytes, text, record.config).unwrap();
            let resolutions = decoded
                .module
                .facts
                .dependencies
                .iter()
                .map(|d| {
                    (
                        d.id,
                        DependencyTarget::File(normalize(
                            &path.parent().unwrap().join(&d.specifier),
                        )),
                    )
                })
                .collect();
            OwnedSourceFileInput::from_parsed(
                path.clone(),
                decoded.to_parsed(text).unwrap(),
                resolutions,
            )
        })
        .collect::<Vec<_>>();
    let graph = SourceGraph::from_owned(inputs);
    let policy = EffectPolicy::default();
    let plan = partition(
        &graph,
        &[RootSeed {
            id: RootId::Entry("entry.mjs".into()),
            evaluation_roots: vec![graph.file_id(Path::new("entry.mjs")).unwrap()],
            binding_roots: vec![],
        }],
        &policy,
    )
    .unwrap();
    let output = esm(&graph, &plan, &policy, &source).unwrap();
    assert_eq!(output.backend, Backend::Native);
    let (_, cold) = build(&source, &[("entry.mjs", false)], false);
    assert_eq!(output.files, cold.files);
    assert_eq!(output.async_modules, cold.async_modules);
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"async once\ntrue 3\n");
}

#[test]
fn require_of_direct_transitive_and_cyclic_async_esm_has_source_diagnostics() {
    for target in ["leaf.mjs", "barrel.mjs", "a.mjs"] {
        let source = sources(&[
            (
                "entry.cjs",
                &format!("module.exports = () => require('./{target}');"),
            ),
            ("barrel.mjs", "export * from './leaf.mjs';"),
            ("leaf.mjs", "export const value = await Promise.resolve(1);"),
            ("a.mjs", "import './b.mjs'; export const value = 1;"),
            ("b.mjs", "import './a.mjs'; import './barrel.mjs';"),
        ]);
        let graph = graph(&source, false);
        let policy = EffectPolicy::default();
        let plan = partition(
            &graph,
            &[RootSeed {
                id: RootId::Entry("entry.cjs".into()),
                evaluation_roots: vec![graph.file_id(Path::new("entry.cjs")).unwrap()],
                binding_roots: vec![],
            }],
            &policy,
        )
        .unwrap();
        let error = format!("{:#}", esm(&graph, &plan, &policy, &source).unwrap_err());
        assert!(
            error.contains("entry.cjs")
                && error.contains(target)
                && error.contains("require of async ESM")
                && error.contains("leaf.mjs")
                && error.contains("bytes"),
            "{error}"
        );
    }
    compare(
        &sources(&[
            (
                "entry.cjs",
                "const ns = require('./sync.mjs'); ns.later().then(ns => console.log(ns.value));",
            ),
            (
                "sync.mjs",
                "export async function later() { return import('./async.mjs'); }",
            ),
            (
                "async.mjs",
                "export const value = await Promise.resolve(8);",
            ),
        ]),
        "entry.cjs",
    );
}

#[test]
fn split_lexical_scopes_keep_mutation_and_initialization_order_across_roots() {
    let sources = sources(&[
        ("a.mjs", "export {x} from './shared.mjs';"),
        ("b.mjs", "export {bump, read} from './shared.mjs';"),
        ("shared.mjs", "export let x = 1; console.log('shared'); export function bump() { x++; } export function read() { return x; }"),
    ]);
    let (plan, output) = build(&sources, &[("a.mjs", true), ("b.mjs", true)], false);
    let module = PathBuf::from("shared.mjs");
    let declaration = NodeId::Segment {
        module: module.clone(),
        index: 0,
    };
    let mutator = NodeId::Segment { module, index: 2 };
    assert_ne!(plan.chunk_for(&declaration), plan.chunk_for(&mutator));
    let mut files = output.files.clone();
    files.insert(
        "observe.mjs".into(),
        observation(
            &output,
            r#"
const a = await import('./a.mjs');
const b = await import('./b.mjs');
console.log(a.x, b.read()); b.bump(); console.log(a.x, b.read());
"#,
        ),
    );
    let result = node(&files, Path::new("observe.mjs"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "shared\n1 1\n2 2\n"
    );
}

#[test]
fn canonical_native_namespaces_and_once_only_evaluation_survive_entry_facades() {
    let sources = sources(&[
        (
            "a.mjs",
            "console.log('a'); export let value = 1; export function bump() { value++; }",
        ),
        ("b.mjs", "export * as original from './a.mjs';"),
    ]);
    let (_, output) = build(&sources, &[("a.mjs", true), ("b.mjs", true)], false);
    let mut files = output.files.clone();
    files.insert("observe.mjs".into(), observation(&output, r#"
const [a, again, b] = await Promise.all([import('./a.mjs'), import('./a.mjs'), import('./b.mjs')]);
console.log(a === again, a === b.original, Object.isExtensible(a), Object.prototype.toString.call(a));
a.bump(); console.log(again.value, b.original.value);
try { a.value = 9; } catch (error) { console.log(error.name); }
"#));
    let result = node(&files, Path::new("observe.mjs"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "a\ntrue true false [object Module]\n2 2\nTypeError\n"
    );
}

#[test]
fn cross_chunk_cycle_keeps_forward_tdz_and_hoisted_function_instantiation() {
    let sources = sources(&[
        ("a.mjs", "import './cycle.mjs'; export let value = 1; export function hoisted() { return 7; } export function unusedByOtherEntry() {}"),
        ("cycle.mjs", "import {value, hoisted} from './a.mjs'; console.log(hoisted()); console.log(value);"),
        ("second.mjs", "import './a.mjs';"),
    ]);
    let (plan, output) = build(&sources, &[("a.mjs", true), ("second.mjs", false)], false);
    assert!(plan.chunks.len() > 1);
    let baseline = node(&sources, Path::new("a.mjs"));
    let bundled = node(&output.files, &output.entries["a.mjs"]);
    assert!(!baseline.status.success() && !bundled.status.success());
    assert_eq!(baseline.stdout, bundled.stdout);
    assert_eq!(String::from_utf8(bundled.stdout).unwrap(), "7\n");
    assert!(String::from_utf8_lossy(&bundled.stderr).contains("ReferenceError"));
    assert!(String::from_utf8_lossy(&bundled.stderr).contains("before initialization"));
}

#[test]
fn native_low_roots_preserve_order_without_inventing_a_public_namespace() {
    let source = sources(&[
        ("a.mjs", "console.log('a'); export const a = 1;"),
        ("b.mjs", "console.log('b'); export const b = 2;"),
        ("selected.mjs", "import {value} from './dep.mjs'; export const selected = console.log('selected', value); const privateValue = console.log('private', value);"),
        ("dep.mjs", "export const value = 7;"),
        ("opaque.cjs", "console.log('opaque'); module.exports = 9;"),
    ]);
    let graph = graph(&source, false);
    let file = |name| graph.file_id(Path::new(name)).unwrap();
    let roots = [
        RootSeed {
            id: RootId::Entry("combined".into()),
            evaluation_roots: vec![file("b.mjs"), file("a.mjs")],
            binding_roots: vec![
                GraphNode::Segment(SegmentKey::new(file("selected.mjs"), 1)),
                GraphNode::Segment(SegmentKey::new(file("selected.mjs"), 2)),
            ],
        },
        RootSeed {
            id: RootId::Entry("bindings".into()),
            evaluation_roots: vec![],
            binding_roots: vec![
                GraphNode::Module(file("a.mjs")),
                GraphNode::Module(file("b.mjs")),
                GraphNode::Module(file("opaque.cjs")),
            ],
        },
    ];
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    let output = esm(&graph, &plan, &policy, &source).unwrap();
    assert_eq!(output.backend, Backend::Native);
    let mut files = output.files.clone();
    files.insert("observe.mjs".into(), observation(&output,
        "const ns = await import('./combined'); console.log(JSON.stringify(Object.keys(ns))); const bindings = await import('./bindings'); console.log(JSON.stringify(Object.keys(bindings)));"));
    let actual = node(&files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        actual.stdout,
        b"b\na\nselected 7\nprivate 7\n[]\nopaque\n[]\n"
    );
}

#[test]
fn native_binding_only_roots_keep_direct_dependencies_and_retained_public_names() {
    for (index, expected) in [
        (1, "private 7\n[\"publicValue\"]\n7\n"),
        (2, "private 7\n[]\nundefined\n"),
    ] {
        let source = sources(&[
            ("dep.mjs", "export const value = 7;"),
            ("selected.mjs", "import {value} from './dep.mjs'; export const publicValue = 7; const privateValue = console.log('private', value);"),
        ]);
        let graph = graph(&source, false);
        let file = graph.file_id(Path::new("selected.mjs")).unwrap();
        let policy = EffectPolicy::default();
        let plan = partition(
            &graph,
            &[RootSeed {
                id: RootId::Entry("selected".into()),
                evaluation_roots: vec![],
                binding_roots: vec![GraphNode::Segment(SegmentKey::new(file, index))],
            }],
            &policy,
        )
        .unwrap();
        let output = esm(&graph, &plan, &policy, &source).unwrap();
        assert_eq!(output.backend, Backend::Native);
        let mut files = output.files.clone();
        files.insert("observe.mjs".into(), observation(&output,
            "const ns = await import('./selected'); console.log(JSON.stringify(Object.keys(ns))); console.log(ns.publicValue);"));
        let actual = node(&files, Path::new("observe.mjs"));
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(String::from_utf8(actual.stdout).unwrap(), expected);
    }
}

#[test]
fn native_entry_aliases_empty_identities_and_evaluation_only_surfaces() {
    let source = sources(&[
        ("empty.mjs", ""),
        (
            "entry.mjs",
            "export const observed = (console.log('once'), {});",
        ),
    ]);
    let graph = graph(&source, false);
    let file = graph.file_id(Path::new("entry.mjs")).unwrap();
    let empty = graph.file_id(Path::new("empty.mjs")).unwrap();
    let roots = [
        RootSeed {
            id: RootId::Entry("first".into()),
            evaluation_roots: vec![file],
            binding_roots: vec![GraphNode::Module(file)],
        },
        RootSeed {
            id: RootId::Entry("__proto__".into()),
            evaluation_roots: vec![file, file],
            binding_roots: vec![GraphNode::Module(file)],
        },
        RootSeed {
            id: RootId::Entry("evaluation".into()),
            evaluation_roots: vec![file],
            binding_roots: vec![],
        },
        RootSeed {
            id: RootId::Entry("empty".into()),
            evaluation_roots: vec![empty],
            binding_roots: vec![GraphNode::Module(empty)],
        },
        RootSeed {
            id: RootId::Entry("nothing".into()),
            evaluation_roots: vec![empty],
            binding_roots: vec![],
        },
    ];
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    let output = esm(&graph, &plan, &policy, &source).unwrap();
    assert_eq!(output.backend, Backend::Native);
    let mut files = output.files.clone();
    files.insert("observe.mjs".into(), observation(&output, r#"
const a = await import('./first'), b = await import('./__proto__');
const evaluation = await import('./evaluation'), empty = await import('./empty'), nothing = await import('./nothing');
console.log(a === b, a.observed === b.observed);
console.log(JSON.stringify(Object.keys(evaluation)), JSON.stringify(Object.keys(empty)), JSON.stringify(Object.keys(nothing)), empty !== nothing);
"#));
    let actual = node(&files, Path::new("observe.mjs"));
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(actual.stdout, b"once\ntrue true\n[] [] [] true\n");
}

#[test]
fn logical_grouping_does_not_evaluate_later_modules_after_a_throw() {
    let sources = sources(&[
        ("a.mjs", "console.log('a'); throw new Error('stop');"),
        ("b.mjs", "console.log('WRONG');"),
    ]);
    let graph = graph(&sources, false);
    let a = graph.file_id(Path::new("a.mjs")).unwrap();
    let b = graph.file_id(Path::new("b.mjs")).unwrap();
    let roots = [RootSeed {
        id: RootId::Entry("combined".into()),
        evaluation_roots: vec![a, b],
        binding_roots: vec![],
    }];
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    assert_eq!(plan.chunks.len(), 1);
    let output = esm(&graph, &plan, &policy, &sources).unwrap();
    let mut files = output.files.clone();
    files.insert(
        "observe.mjs".into(),
        observation(
            &output,
            "try { await import('./combined'); } catch { console.log('caught'); }",
        ),
    );
    let result = node(&files, Path::new("observe.mjs"));
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "a\ncaught\n");
}

#[test]
fn linked_files_apply_pure_recipes_and_preserve_utf8_hashbang_directives() {
    let sources = sources(&[
        ("entry.mjs", "import './dep.mjs';"),
        ("dep.mjs", "#!/usr/bin/env node\n// legal é\n'use strict';\nfunction dead() { return 'must disappear'; }\nconst obj = {get method() { console.log('get'); return undefined; }}; function arg() { console.log('arg'); }\n/*#__PURE__*/ obj?.method(arg());\n//# sourceMappingURL=obsolete.map"),
    ]);
    let (_, output) = build(&sources, &[("entry.mjs", true)], false);
    assert_eq!(output.files.len(), 1);
    let result = node(&output.files, &output.entries["entry.mjs"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "get\narg\n");
    assert!(!output.files.values().any(|s| s.contains("must disappear")));
    assert!(!output.files.values().any(|s| s.contains("obsolete.map")));
}

#[test]
fn separate_bundle_directories_do_not_share_native_module_state() {
    let first = sources(&[
        (
            "entry.mjs",
            "import counter from './counter.cjs'; export let value = counter.value;",
        ),
        ("counter.cjs", "module.exports = {value: 1};"),
    ]);
    let second = sources(&[
        (
            "entry.mjs",
            "import counter from './counter.cjs'; export let value = counter.value;",
        ),
        ("counter.cjs", "module.exports = {value: 2};"),
    ]);
    let (_, first) = build(&first, &[("entry.mjs", true)], false);
    let (_, second) = build(&second, &[("entry.mjs", true)], false);
    let observer = format!(
        "const one = () => import({}); const two = () => import({}); const a = await one(), b = await two(); console.log(a.value, b.value, a === b, a === await one());",
        quote(&format!("./one/{}", first.entries["entry.mjs"].display())),
        quote(&format!("./two/{}", second.entries["entry.mjs"].display())),
    );
    let mut files = BTreeMap::new();
    for (directory, output) in [("one", first), ("two", second)] {
        for (path, source) in output.files {
            files.insert(Path::new(directory).join(path), source);
        }
    }
    files.insert("observe.mjs".into(), observer);
    let result = node(&files, Path::new("observe.mjs"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "1 2 false true\n"
    );
}

#[test]
fn lazy_commonjs_bridges_survive_native_compatible_intrinsic_mutations() {
    for mutation in [
        "Object.getOwnPropertyNames = fail",
        "Object.create = fail",
        "Object.defineProperty = fail",
        "Object.freeze = fail",
        "Object.assign = fail",
        "globalThis.Object = fail",
        "globalThis.Symbol = fail",
        "Reflect.apply = fail",
        "Array.prototype[Symbol.iterator] = fail",
    ] {
        let source = sources(&[
            (
                "entry.mjs",
                &format!(
                    "const fail = () => {{throw Error('patched');}}; {mutation}; \
                     console.log((await import('./dep.cjs')).default.value);"
                ),
            ),
            ("dep.cjs", "module.exports = {value: 42};"),
        ]);
        let (_, output) = build(&source, &[("entry.mjs", false)], false);
        for result in [
            node(&source, Path::new("entry.mjs")),
            node(&output.files, &output.entries["entry.mjs"]),
        ] {
            assert!(
                result.status.success(),
                "{mutation}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(result.stdout, b"42\n", "{mutation}");
        }
    }
}

#[test]
fn commonjs_cycle_diagnostics_use_captured_intrinsics() {
    let source = sources(&[
        (
            "entry.mjs",
            "Object.assign = () => {throw Error('patched');}; \
             try { await import('./cycle.cjs'); } catch (error) { console.log(error.code); }",
        ),
        ("cycle.cjs", "exports.value = 1; require('./bridge.mjs');"),
        (
            "bridge.mjs",
            "import value from './cycle.cjs'; export {value};",
        ),
    ]);
    let output = compare(&source, "entry.mjs");
    assert_eq!(
        node(&output.files, &output.entries["entry.mjs"]).stdout,
        b"ERR_REQUIRE_CYCLE_MODULE\n"
    );
}

#[test]
fn later_entries_load_after_user_code_changes_public_builtins() {
    let sources = sources(&[
        ("first.mjs", "const fail = () => { throw new Error('public builtin changed'); }; JSON.parse = fail; Object.hasOwn = fail; globalThis.URL = fail; Map.prototype.get = fail; Map.prototype.has = fail; Map.prototype.set = fail;"),
        ("second.mjs", "import * as ns from './constant.cjs'; export const value = ns.default;"),
        ("constant.cjs", "module.exports = 42;"),
    ]);
    let (_, output) = build(
        &sources,
        &[("first.mjs", false), ("second.mjs", true)],
        false,
    );
    let mut files = output.files.clone();
    files.insert(
        "observe.mjs".into(),
        observation(
            &output,
            "await import('./first.mjs'); console.log((await import('./second.mjs')).value);",
        ),
    );
    let result = node(&files, Path::new("observe.mjs"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "42\n");
}

#[test]
fn stale_and_missing_sources_or_external_policies_fail_without_staging() {
    let sources = sources(&[("entry.mjs", "console.log('entry');")]);
    let graph = graph(&sources, false);
    let file = graph.file_id(Path::new("entry.mjs")).unwrap();
    let roots = [RootSeed {
        id: RootId::Entry("__proto__".into()),
        evaluation_roots: vec![file],
        binding_roots: vec![],
    }];
    let policy = EffectPolicy::default();
    let plan = partition(&graph, &roots, &policy).unwrap();
    assert!(esm(
        &graph,
        &plan,
        &policy.clone().with_pure_annotations(false),
        &sources
    )
    .is_err());
    assert!(esm(&graph, &plan, &policy, &BTreeMap::new())
        .unwrap_err()
        .to_string()
        .contains("entry.mjs"));
    let output = esm(&graph, &plan, &policy, &sources).unwrap();
    let result = node(&output.files, &output.entries["__proto__"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "entry\n");
    let mut input = crate::tests::input("external.mjs", "import 'node:fs';");
    for target in input.resolutions.values_mut() {
        *target = DependencyTarget::External("node:fs".into());
    }
    let graph = SourceGraph::from_owned([input]);
    let roots = [RootSeed {
        id: RootId::Entry("external".into()),
        evaluation_roots: vec![0],
        binding_roots: vec![],
    }];
    let plan = partition(&graph, &roots, &policy).unwrap();
    let sources = BTreeMap::from([("external.mjs".into(), "import 'node:fs';".into())]);
    let output = esm(&graph, &plan, &policy, &sources).unwrap();
    assert!(node(&output.files, &output.entries["external"])
        .status
        .success());
}

fn read_sources(directory: &Path, prefix: &Path, sources: &mut BTreeMap<PathBuf, String>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            read_sources(&entry.path(), &prefix.join(entry.file_name()), sources);
        } else if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "js" || ext == "mjs" || ext == "cjs")
        {
            sources.insert(
                prefix.join(entry.file_name()),
                std::fs::read_to_string(entry.path()).unwrap(),
            );
        }
    }
}

#[test]
fn portable_fixture_observations_match_native_and_manifest_expectations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bundler");
    for fixture in [
        "esbuild/reexport-cycle",
        "esbuild/shared-entrypoint-identity",
        "esbuild/unused-import-side-effects",
        "rspack/esm-effect-order",
        "rspack/esm-live-binding",
        "rspack/cjs-default-export",
        "rspack/cjs-named-reexport",
        "rspack/cjs-self-cycle",
        "rspack/dynamic-import-cjs",
        "esbuild/cjs-sync-async-once",
        "esbuild/esm-sync-async",
        "esbuild/dynamic-import-tla-order",
    ] {
        let directory = root.join(fixture);
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(directory.join("case.json")).unwrap())
                .unwrap();
        let mut sources = BTreeMap::new();
        read_sources(&directory.join("files"), Path::new(""), &mut sources);
        // Sources run in an isolated directory with the original module policy.
        let mut baseline_files = sources.clone();
        baseline_files.insert(
            PathBuf::from("package.json"),
            serde_json::json!({"type": manifest["moduleType"]}).to_string(),
        );
        let entries: Vec<_> = manifest["entryPoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry.as_str().unwrap(), true))
            .collect();
        let format = if manifest["moduleType"] == "commonjs" {
            ModuleFormat::CommonJs
        } else {
            ModuleFormat::Esm
        };
        let (_, output) = build_with_format(&sources, &entries, false, format);
        let mut observed = String::new();
        for (entry, _) in entries {
            let baseline = node(&baseline_files, Path::new(entry));
            let bundled = node(&output.files, &output.entries[entry]);
            assert!(
                baseline.status.success(),
                "{fixture}: {}",
                String::from_utf8_lossy(&baseline.stderr)
            );
            assert!(
                bundled.status.success(),
                "{fixture}: {}",
                String::from_utf8_lossy(&bundled.stderr)
            );
            assert_eq!(bundled.stdout, baseline.stdout, "{fixture}/{entry}");
            observed.push_str(std::str::from_utf8(&bundled.stdout).unwrap());
        }
        let expected = manifest["expectations"]["stdout"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| format!("{}\n", line.as_str().unwrap()))
            .collect::<String>();
        assert_eq!(observed, expected, "{fixture}");
    }
}
