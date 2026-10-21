use super::*;

fn precise(
    input: &BTreeMap<PathBuf, String>,
    public: &[&str],
    wire: bool,
) -> (SourceGraph, ChunkPlan, Artifacts) {
    let graph = graph_with_storage(input, wire, ModuleFormat::Esm, wire);
    let roots = public
        .iter()
        .map(|path| {
            let file = graph.file_id(Path::new(path)).unwrap();
            RootSeed {
                id: RootId::Entry((*path).into()),
                evaluation_roots: vec![file],
                binding_roots: vec![GraphNode::Module(file)],
            }
        })
        .collect::<Vec<_>>();
    let policy = EffectPolicy::default();
    let plan = crate::chunks::partition_with_options(
        &graph,
        &roots,
        &policy,
        &crate::build::OutputOptions::default(),
    )
    .unwrap();
    assert_eq!(
        plan,
        crate::chunks::partition(&graph, &roots, &policy).unwrap()
    );
    assert!(plan.precise_dynamic_imports);
    let output = esm(&graph, &plan, &policy, input).unwrap();
    (graph, plan, output)
}

fn declaration(
    graph: &SourceGraph,
    input: &BTreeMap<PathBuf, String>,
    file: &str,
    text: &str,
) -> NodeId {
    let path = Path::new(file);
    let file_id = graph.file_id(path).unwrap();
    let (index, _) = graph
        .file_segments(file_id)
        .unwrap()
        .iter()
        .enumerate()
        .find(|(_, segment)| {
            input[path][segment.span.start as usize..segment.span.end as usize].contains(text)
        })
        .unwrap();
    NodeId::Segment {
        module: path.into(),
        index: index as u32,
    }
}

fn matches_source(input: &BTreeMap<PathBuf, String>, output: &Artifacts) {
    let baseline = node(input, Path::new("entry.mjs"));
    let actual = node(&output.files, &output.entries["entry.mjs"]);
    assert!(baseline.status.success(), "{:?}", baseline);
    assert!(actual.status.success(), "{:?}", actual);
    assert_eq!(baseline.stdout, actual.stdout);
}

#[test]
fn star_barrel_occurrences_project_independent_symbol_owners_and_shared_effects() {
    let input = sources(&[
        ("entry.mjs", "globalThis.trace = []; const {a, b} = await import('./barrel.mjs'); const again = await import('./barrel.mjs').then(ns => ns.a); console.log(a(), b(), a === again, trace.join(','));"),
        ("barrel.mjs", "import './effects.mjs'; export * from './bridge.mjs'; trace.push('barrel');"),
        ("bridge.mjs", "export * from './leaf.mjs';"),
        ("effects.mjs", "trace.push('effect');"),
        ("leaf.mjs", "function shared() { return 10; } function onlyA() { return 1; } function onlyB() { return 2; } export function a() { return shared() + onlyA(); } export function b() { return shared() + onlyB(); } export function dead() { return import('./dead.mjs'); }"),
        ("dead.mjs", "throw new Error('dead import');"),
    ]);
    for wire in [false, true] {
        let (graph, plan, output) = precise(&input, &["entry.mjs"], wire);
        let a = declaration(&graph, &input, "leaf.mjs", "export function a");
        let b = declaration(&graph, &input, "leaf.mjs", "export function b");
        let shared = declaration(&graph, &input, "leaf.mjs", "function shared");
        let only_a = declaration(&graph, &input, "leaf.mjs", "function onlyA");
        let only_b = declaration(&graph, &input, "leaf.mjs", "function onlyB");
        let a_roots = &plan.node_roots[&a];
        let b_roots = &plan.node_roots[&b];
        assert_eq!(a_roots.len(), 2);
        assert_eq!(b_roots.len(), 1);
        assert!(a_roots
            .iter()
            .all(|root| matches!(root, RootId::AsyncExport { name, .. } if name == "a")));
        assert!(b_roots
            .iter()
            .all(|root| matches!(root, RootId::AsyncExport { name, .. } if name == "b")));
        let union = a_roots.union(b_roots).cloned().collect::<BTreeSet<_>>();
        assert_eq!(plan.node_roots[&shared], union);
        assert_eq!(plan.node_roots[&only_a], *a_roots);
        assert_eq!(plan.node_roots[&only_b], *b_roots);
        let effect = declaration(&graph, &input, "effects.mjs", "trace.push");
        assert_eq!(plan.node_roots[&effect], union);
        assert_ne!(output.members[&a], output.members[&b]);
        assert_ne!(output.members[&shared], output.members[&a]);
        assert_ne!(output.members[&shared], output.members[&b]);
        assert_eq!(output.members[&a], output.members[&only_a]);
        assert_eq!(output.members[&b], output.members[&only_b]);
        for (node, text) in [
            (&a, "function a("),
            (&b, "function b("),
            (&shared, "function shared("),
            (&only_a, "function onlyA("),
            (&only_b, "function onlyB("),
        ] {
            assert!(output.files[&output.members[node]].contains(text));
            assert_eq!(
                output
                    .files
                    .values()
                    .filter(|body| body.contains(text))
                    .count(),
                1
            );
        }
        let clustered = esm_with_options(
            &graph,
            &plan,
            &EffectPolicy::default(),
            &input,
            &crate::build::OutputOptions {
                min_chunk_bytes: 10_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(clustered.members[&a], clustered.members[&b]);
        assert_ne!(clustered.members[&shared], clustered.members[&a]);
        assert_ne!(clustered.members[&shared], clustered.members[&b]);
        matches_source(&input, &clustered);
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
        let occurrence = plan
            .async_activations
            .values()
            .find(|a| a.roots.len() == 2)
            .unwrap();
        assert_eq!(
            occurrence
                .roots
                .iter()
                .filter(|r| a_roots.contains(*r))
                .count(),
            1
        );
        assert_eq!(
            occurrence
                .roots
                .iter()
                .filter(|r| b_roots.contains(*r))
                .count(),
            1
        );
        matches_source(&input, &output);
    }
}

#[test]
fn selected_and_full_public_consumers_keep_live_values_and_complete_namespaces() {
    let input = sources(&[
        ("entry.mjs", "const {value, change} = await import('./barrel.mjs'); const ns = await import('./barrel.mjs'); console.log(value, Object.keys(ns).join(',')); change(); console.log(ns.value, ns.other());"),
        ("barrel.mjs", "export * from './leaf.mjs';"),
        ("leaf.mjs", "export let value = 1; export function change() { value++; } export function other() { return 42; }"),
    ]);
    let (_, plan, output) = precise(&input, &["entry.mjs", "barrel.mjs"], false);
    assert!(plan
        .roots
        .keys()
        .any(|root| matches!(root, RootId::AsyncExport { .. })));
    assert!(plan
        .roots
        .keys()
        .any(|root| matches!(root, RootId::Async(_))));
    matches_source(&input, &output);
    let mut files = output.files.clone();
    files.insert(PathBuf::from("check.mjs"), format!(
        "import assert from 'node:assert/strict'; const ns = await import('./{}'); assert.deepEqual(Object.keys(ns), ['change','other','value']); ns.change(); assert.equal(ns.value, 2);",
        output.entries["barrel.mjs"].to_string_lossy().replace('\\', "/")
    ));
    let actual = node(&files, Path::new("check.mjs"));
    assert!(actual.status.success(), "{actual:?}");
}

#[test]
fn missing_properties_and_unused_imports_only_activate_original_evaluation() {
    let input = sources(&[
        ("entry.mjs", "globalThis.trace = []; const missing = (await import('./barrel.mjs')).absent; await import('./barrel.mjs').then(ns => ns.absent); await import('./barrel.mjs'); console.log(missing === undefined, trace.join(','));"),
        ("barrel.mjs", "import './effects.mjs'; trace.push('once'); export function spare() { return 3; }"),
        ("effects.mjs", "trace.push('dependency');"),
    ]);
    let (graph, plan, output) = precise(&input, &["entry.mjs"], false);
    assert_eq!(plan.async_activations.len(), 3);
    assert!(
        plan.node_roots[&declaration(&graph, &input, "barrel.mjs", "export function spare")]
            .is_empty()
    );
    matches_source(&input, &output);
    let clustered = esm_with_options(
        &graph,
        &plan,
        &EffectPolicy::default(),
        &input,
        &crate::build::OutputOptions {
            min_chunk_bytes: 10_000,
            ..Default::default()
        },
    )
    .unwrap();
    matches_source(&input, &clustered);
}

#[test]
fn deferred_loaders_keep_occurrence_routes_lazy_and_discover_nested_live_imports() {
    let input = sources(&[
        ("entry.mjs", "globalThis.trace = []; export async function loadA() { return (await import('./barrel.mjs')).a; } export function loadB() { return import('./barrel.mjs').then(ns => ns.b); } console.log('before', trace.length); Promise.all([loadA(), loadB()]).then(async ([a,b]) => console.log(await a(), b(), trace.join(',')));"),
        ("barrel.mjs", "export * from './leaf.mjs'; trace.push('barrel');"),
        ("leaf.mjs", "export async function a() { const {value} = await import('./nested.mjs'); return value; } export function b() { return 2; } export function dead() { return import('./dead.mjs'); }"),
        ("nested.mjs", "trace.push('nested'); export const value = 11;"),
        ("dead.mjs", "throw new Error('unreachable');"),
    ]);
    let (graph, plan, output) = precise(&input, &["entry.mjs"], false);
    assert_eq!(plan.async_activations.len(), 3);
    let a = declaration(&graph, &input, "leaf.mjs", "export async function a");
    let b = declaration(&graph, &input, "leaf.mjs", "export function b");
    assert_ne!(output.members[&a], output.members[&b]);
    assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
    matches_source(&input, &output);
}

#[test]
fn native_selected_tla_rejection_receiver_and_then_assimilation() {
    for (target, consumer) in [
        ("await Promise.resolve(); export const a = 3;", "const {a} = await import('./target.mjs'); console.log(a);"),
        ("globalThis.failure = new Error('same'); await Promise.resolve(); throw failure; export const a = 1; export const b = 2;", "const p = import('./target.mjs').then(ns => ns.a); const q = import('./target.mjs').then(ns => ns.b); const results = await Promise.allSettled([p,q]); console.log(results[0].reason === results[1].reason, results[0].reason === failure);"),
        ("export function a() { return this.b; } export const b = 7;", "console.log((await import('./target.mjs')).a());"),
        ("export function then(resolve) { resolve({a: 9}); } export const a = 1;", "console.log((await import('./target.mjs')).a);"),
    ] {
        let input = sources(&[("entry.mjs", consumer), ("target.mjs", target)]);
        let (_, _, output) = precise(&input, &["entry.mjs"], false);
        matches_source(&input, &output);
    }
}

#[test]
fn precise_selected_esm_export_omits_sibling_commonjs_snapshot_at_runtime() {
    let input = sources(&[
        (
            "entry.mjs",
            "console.log((await import('./barrel.mjs')).b);",
        ),
        (
            "barrel.mjs",
            "import {a} from './dep.cjs'; const b = 2; export {a, b};",
        ),
        ("dep.cjs", "exports.a = 1;"),
    ]);
    for wire in [false, true] {
        let (_, plan, output) = precise(&input, &["entry.mjs"], wire);
        assert!(!plan.node_roots.contains_key(&NodeId::CommonJsProxy {
            module: "dep.cjs".into(),
            name: Some("a".into()),
        }));
        matches_source(&input, &output);
    }
}

#[test]
fn precise_libraries_reject_selected_targets_in_complete_initializer_closure() {
    let input = sources(&[
            ("entry.mjs", "globalThis.trace = []; export const selected = () => import('./shared.mjs').then(ns => ns.value); export const full = async () => (await import('./lib.mjs')).run(); console.log(await selected(), trace.join(','));"),
            ("lib.mjs", "import './one.mjs'; import './two.mjs'; export {run} from './shared.mjs';"),
            ("one.mjs", "trace.push('one');"),
            ("two.mjs", "trace.push('two');"),
            ("shared.mjs", "trace.push('shared'); export const value = 42; export function run() { return value; }"),
        ]);
    for wire in [false, true] {
        let (graph, plan, ordinary) = precise(&input, &["entry.mjs"], wire);
        let output = esm_with_options(
            &graph,
            &plan,
            &EffectPolicy::default(),
            &input,
            &crate::build::OutputOptions {
                min_chunk_bytes: 10_000,
                ..Default::default()
            },
        )
        .unwrap();
        let value = declaration(&graph, &input, "shared.mjs", "export const value");
        let effects = declaration(&graph, &input, "shared.mjs", "trace.push");
        assert_eq!(output.members[&value], ordinary.members[&value]);
        assert_eq!(output.members[&effects], ordinary.members[&effects]);
        matches_source(&input, &output);
        let mut files = output.files.clone();
        files.insert(
                "check.mjs".into(),
                format!(
                    "const entry = await import('./{}'); console.log(await entry.full(), trace.join(',')); console.log(await entry.selected(), trace.join(','));",
                    output.entries["entry.mjs"].to_string_lossy().replace('\\', "/")
                ),
            );
        let actual = node(&files, Path::new("check.mjs"));
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout).replace("\r\n", "\n"),
            "42 shared\n42 shared,one,two\n42 shared,one,two\n"
        );
    }
}
