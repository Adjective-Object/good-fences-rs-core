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
    let mut inputs: Vec<_> = sources
        .iter()
        .map(|(path, source)| {
            let parsed =
                parse_module(&logger::StdioLogger::new(), source, path, ModuleFormat::Esm).unwrap();
            let resolutions = parsed
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
    let graph = graph(sources, reverse);
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
fn rejected_partition_and_loading_cases_return_source_diagnostics() {
    for (items, entries, diagnostic) in [
        (
            vec![
                (
                    "entry.mjs",
                    "export function later() { return import('./dep.mjs'); }",
                ),
                ("dep.mjs", "export const x = 1;"),
            ],
            vec![("entry.mjs", true)],
            "awaits P7/P8",
        ),
        (
            vec![
                ("entry.mjs", "import('./dep.mjs').then(console.log);"),
                ("dep.mjs", "export const x = 1;"),
            ],
            vec![("entry.mjs", true)],
            "awaits P7/P8",
        ),
    ] {
        let sources = sources(&items);
        let graph = graph(&sources, false);
        let roots: Vec<_> = entries
            .iter()
            .map(|(name, _)| {
                let file = graph.file_id(Path::new(name)).unwrap();
                RootSeed {
                    id: RootId::Entry((*name).into()),
                    evaluation_roots: vec![file],
                    binding_roots: vec![GraphNode::Module(file)],
                }
            })
            .collect();
        let policy = EffectPolicy::default();
        let plan = partition(&graph, &roots, &policy).unwrap();
        let error = format!("{:#}", esm(&graph, &plan, &policy, &sources).unwrap_err());
        assert!(
            error.contains(diagnostic) && error.contains("bytes"),
            "{error}"
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
    let first = sources(&[("entry.mjs", "export let value = 1;")]);
    let second = sources(&[("entry.mjs", "export let value = 2;")]);
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
        ("second.mjs", "export const value = 42;"),
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
            .is_some_and(|ext| ext == "js" || ext == "mjs")
        {
            sources.insert(
                prefix.join(entry.file_name()),
                std::fs::read_to_string(entry.path()).unwrap(),
            );
        }
    }
}

#[test]
fn portable_esm_fixture_observations_match_native_and_manifest_expectations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bundler");
    for fixture in [
        "esbuild/reexport-cycle",
        "esbuild/shared-entrypoint-identity",
        "esbuild/unused-import-side-effects",
        "rspack/esm-effect-order",
        "rspack/esm-live-binding",
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
            "{\"type\":\"module\"}".into(),
        );
        let entries: Vec<_> = manifest["entryPoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry.as_str().unwrap(), true))
            .collect();
        let (_, output) = build(&sources, &entries, false);
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
