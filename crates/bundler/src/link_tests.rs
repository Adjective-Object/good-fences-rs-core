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
            OwnedSourceFileInput {
                path: path.clone(),
                segments: parsed.segments,
                facts: parsed.facts,
                resolutions,
            }
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

fn compare(sources: &BTreeMap<PathBuf, String>, entry: &str) -> Artifacts {
    let (_, output) = build(sources, &[(entry, false)], false);
    let baseline = node(sources, Path::new(entry));
    let bundled = node(&output.files, &output.entries[entry]);
    assert_eq!(
        bundled.status.success(),
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&bundled.stderr)
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
fn commonjs_native_cache_cycles_aliases_replacement_and_retry() {
    compare(&sources(&[
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
    let graph = SourceGraph::from_owned([OwnedSourceFileInput {
        path: "dep.js".into(),
        segments: parsed.segments,
        facts: parsed.facts,
        resolutions: Default::default(),
    }]);
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
    output.files.insert(
        "observe.mjs".into(),
        "import {loadEntry} from './runtime.mjs'; console.log((await loadEntry('main')).default);"
            .into(),
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
fn commonjs_exact_proxy_sets_share_one_native_registry_across_entries() {
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
    let mut files = output.files;
    files.insert("observe.mjs".into(), r#"
import {loadEntry} from './runtime.mjs';
const lazy = await loadEntry('lazy.cjs');
console.log('not initialized');
const [a, b, again] = await Promise.all([loadEntry('a.mjs'), loadEntry('b.mjs'), loadEntry('a.mjs')]);
const common = lazy.default();
console.log(a === again, a.a === common.a, b.b === common.b, common.a.partial);
"#.into());
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
    output.files.insert("observe.mjs".into(), "import {loadEntry} from './runtime.mjs'; const entry = await loadEntry('entry.mjs'); console.log(entry.ns === await loadEntry('dep.cjs'));".into());
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
fn native_cycle_instantiation_distinguishes_functions_var_and_lexical_tdz() {
    let output = compare(&sources(&[
        ("entry.mjs", "import {read, bump, value} from './a.mjs'; console.log(read(), value); bump(); console.log(read(), value);"),
        ("a.mjs", "import {initial} from './b.mjs'; export var value = initial + 1; export function read() { return value; } export function bump() { value++; }"),
        ("b.mjs", "import {read, value} from './a.mjs'; console.log(typeof read, value, read()); export var initial = 4;"),
    ]), "entry.mjs");
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
fn exact_cross_chunk_artifacts_share_identity_without_merging_or_eager_eval() {
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
    assert_ne!(output.members[&first], output.members[&second]);
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
    assert_eq!(output.async_facades.len(), 5);
    assert!(!output.modules.contains_key(Path::new("dead.mjs")));
    assert_eq!(
        output
            .files
            .keys()
            .filter(|p| p.as_path() == Path::new("runtime.mjs"))
            .count(),
        1
    );
}

#[test]
fn dynamic_commonjs_uses_the_same_conservative_namespace_as_static_and_entry_loads() {
    let source = sources(&[
        ("entry.mjs", "import * as sync from './dep.cjs'; export async function check() { const [a,b] = await Promise.all([import('./dep.cjs'), import('./dep.cjs')]); console.log(a === b, a === sync, a.default === sync.default, a.computedName); a.bump(); console.log(a.computedName); return a; }"),
        ("dep.cjs", "console.log('factory once'); let value = 1; module.exports = {['computed' + 'Name']: value, bump() { module.exports.computedName = ++value; }};"),
    ]);
    let (_, mut output) = build(&source, &[("entry.mjs", true), ("dep.cjs", false)], false);
    output.files.insert("observe.mjs".into(), "import {loadEntry} from './runtime.mjs'; const entry = await loadEntry('entry.mjs'); const ns = await entry.check(); console.log(ns === await loadEntry('dep.cjs'));".into());
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
fn dynamic_payload_load_and_parse_failures_reject_without_eager_execution() {
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
        let (_, mut output) = build(&source, &[("entry.mjs", true)], false);
        let paths = output.module_payloads[Path::new("dep.mjs")].clone();
        let mut restore = BTreeMap::new();
        for path in paths {
            let original = output.files.remove(&path).unwrap();
            restore.insert(path.clone(), original.clone());
            if malformed {
                let mut payload: BTreeMap<String, String> =
                    serde_json::from_str(&original).unwrap();
                for piece in payload.values_mut() {
                    *piece = "invalid syntax !!!".into();
                }
                output
                    .files
                    .insert(path, serde_json::to_string(&payload).unwrap());
            }
        }
        output.files.insert("observe.mjs".into(), format!(
            "import {{writeFileSync}} from 'node:fs'; import {{loadEntry}} from './runtime.mjs'; const entry = await loadEntry('entry.mjs'); console.log('entry ready'); const results = await Promise.allSettled([entry.later(), entry.later()]); console.log(results.every(r => r.status === 'rejected'), results[0].reason.name); const restore = {}; for (const [path,source] of Object.entries(restore)) writeFileSync(new URL(path, import.meta.url), source); try {{ await entry.later(); console.log('retry succeeded'); }} catch(e) {{ console.log('rejected', e.name); }}",
            serde_json::to_string(&restore).unwrap()
        ));
        let actual = node(&output.files, Path::new("observe.mjs"));
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(
            actual.stdout,
            if malformed {
                b"entry ready\ntrue SyntaxError\nrejected SyntaxError\n".as_slice()
            } else {
                b"entry ready\ntrue Error\nunexpected evaluation\nretry succeeded\n".as_slice()
            }
        );
    }
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
        r#"
import {loadEntry} from './runtime.mjs';
const a = await loadEntry('a.mjs');
const b = await loadEntry('b.mjs');
console.log(a.x, b.read()); b.bump(); console.log(a.x, b.read());
"#
        .into(),
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
    let mut files = output.files;
    files.insert("observe.mjs".into(), r#"
import {loadEntry} from './runtime.mjs';
const [a, again, b] = await Promise.all([loadEntry('a.mjs'), loadEntry('a.mjs'), loadEntry('b.mjs')]);
console.log(a === again, a === b.original, Object.isExtensible(a), Object.prototype.toString.call(a));
a.bump(); console.log(again.value, b.original.value);
try { a.value = 9; } catch (error) { console.log(error.name); }
"#.into());
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
fn loading_payloads_never_evaluates_another_module_in_the_same_chunk() {
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
    let mut files = output.files;
    files.insert("observe.mjs".into(), "import {loadEntry} from './runtime.mjs'; try { await loadEntry('combined'); } catch { console.log('caught'); }".into());
    let result = node(&files, Path::new("observe.mjs"));
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "a\ncaught\n");
}

#[test]
fn linked_payloads_apply_pure_recipes_and_preserve_utf8_hashbang_directives() {
    let sources = sources(&[
        ("entry.mjs", "import './dep.mjs';"),
        ("dep.mjs", "#!/usr/bin/env node\n// legal é\n'use strict';\nfunction dead() { return 'must disappear'; }\nconst obj = {get method() { console.log('get'); return undefined; }}; function arg() { console.log('arg'); }\n/*#__PURE__*/ obj?.method(arg());\n//# sourceMappingURL=obsolete.map"),
    ]);
    let (_, output) = build(&sources, &[("entry.mjs", false)], false);
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
fn separate_bundle_instances_do_not_share_virtual_module_state() {
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
    let mut files = BTreeMap::new();
    for (directory, output) in [("one", first), ("two", second)] {
        for (path, source) in output.files {
            files.insert(Path::new(directory).join(path), source);
        }
    }
    files.insert(
        "observe.mjs".into(),
        r#"
import {loadEntry as one} from './one/runtime.mjs';
import {loadEntry as two} from './two/runtime.mjs';
const a = await one('entry.mjs'), b = await two('entry.mjs');
console.log(a.value, b.value, a === b, a === await one('entry.mjs'));
"#
        .into(),
    );
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
    let mut files = output.files;
    files.insert("observe.mjs".into(), "import {loadEntry} from './runtime.mjs'; await loadEntry('first.mjs'); console.log((await loadEntry('second.mjs')).value);".into());
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
    let error = esm(&graph, &plan, &policy, &sources)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("external.mjs") && error.contains("bytes") && error.contains("policy"),
        "{error}"
    );
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
