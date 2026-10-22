use super::*;
use crate::{
    build::OutputOptions,
    lazy_modules::{certify, ConstructorExport, LazyRuntime},
};
use ast_segmenter::emission::LazyWrapperKind;

const IMPORTS: &str = "import {Module as M, Read as R, Action as A} from './runtime-barrel.mjs';";
const RUNTIME: &str = "export class ModuleCtor { constructor(load, options) { this.load = load; this.options = options; } }
    export class ImportCtor { constructor(module, get, shouldGovern) { this.module = module; this.get = get; this.shouldGovern = shouldGovern; } }
    export class ActionCtor { constructor(module, get, options) { this.module = module; this.get = get; this.options = options; } }";

// Authored test contract; deliberately retains real wrapper behavior.
const BEHAVIOR_RUNTIME: &str = "
export class ModuleCtor {
    constructor(load, options) { this.load = load; this.options = options; this.loaded = false; }
    getValue() {
        if (!this.pending) this.pending = this.load().then(ns => {
            this.loaded = true;
            if (this.options?.initializer) this.options.initializer(ns);
            if (this.options?.onLoad) this.options.onLoad();
            return ns;
        }).catch(error => { this.pending = undefined; throw error; });
        return this.pending;
    }
}
export class ImportCtor {
    constructor(module, getter, govern, extra) {
        this.module = module; this.getter = getter; this.govern = govern;
        this.extra = extra; this.calls = 0;
    }
    async getValue() {
        this.calls++;
        if (this.govern) this.govern();
        return this.getter(await this.module.getValue());
    }
}
export class ActionCtor {
    constructor(module, getter, options) {
        this.module = module; this.getter = getter; this.options = options; this.calls = 0;
    }
    async invoke(...args) {
        this.calls++;
        const action = this.getter(await this.module.getValue());
        return action(...args);
    }
}";

fn emit_lazy(
    input: &BTreeMap<PathBuf, String>,
    entries: &[(&str, bool)],
    wire: bool,
) -> (SourceGraph, ChunkPlan, Artifacts) {
    let graph = graph_with_storage(input, wire, ModuleFormat::Esm, wire);
    let options = OutputOptions {
        lazy_runtimes: vec![runtime()],
        ..Default::default()
    };
    let roots: Vec<_> = entries
        .iter()
        .map(|(path, namespace)| {
            let file = graph.file_id(Path::new(path)).unwrap();
            RootSeed {
                id: RootId::Entry((*path).into()),
                evaluation_roots: vec![file],
                binding_roots: if *namespace {
                    vec![GraphNode::Module(file)]
                } else {
                    vec![]
                },
            }
        })
        .collect();
    let plan =
        crate::chunks::partition_with_options(&graph, &roots, &EffectPolicy::default(), &options)
            .unwrap();
    let parses = ast_segmenter::module_parse_count();
    let emitted =
        esm_with_options(&graph, &plan, &EffectPolicy::default(), input, &options).unwrap();
    assert_eq!(ast_segmenter::module_parse_count(), parses);
    (graph, plan, emitted)
}

fn observe_lazy(mut emitted: Artifacts, observer: &str) {
    let mut script = "import assert from 'node:assert/strict'; globalThis.trace = [];\n".to_owned();
    for (index, (entry, path)) in emitted.entries.iter().enumerate() {
        script.push_str(&format!(
            "const load{index} = () => import({}); // {entry}\n",
            quote(&format!("./{}", path.to_string_lossy().replace('\\', "/")))
        ));
    }
    script.push_str(observer);
    emitted.files.insert("observe.mjs".into(), script);
    let result = node(&emitted.files, Path::new("observe.mjs"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn lazy_default_low_level_keeps_original_allocation_namespace_and_loaded_state() {
    let mut input = input(
        "
        const originalModule = new M(() => import('./target.mjs'));
        export const a = new R(originalModule, ns => ns.first);
        export const b = new R(originalModule, ns => ns.second);",
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    for wire in [false, true] {
        let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
        let policy = EffectPolicy::default();
        let plan = crate::chunks::partition(&graph, &roots(&graph), &policy).unwrap();
        assert!(plan.lazy_modules.modules().is_empty());
        let emitted = esm(&graph, &plan, &policy, &input).unwrap();
        assert!(emitted
            .files
            .values()
            .any(|code| code.contains("originalModule")));
        observe_lazy(
            emitted,
            "
            const {a,b} = await load0();
            assert.equal(a.module, b.module);
            assert.equal(b.module.loaded, false);
            assert.equal((await a.getValue())(), 1);
            assert.equal(b.module.loaded, true);
            assert.deepEqual(Object.keys(await a.module.getValue()), ['first','second']);
        ",
        );
    }
}

#[test]
fn lazy_emission_injected_reads_avoid_nested_constructor_and_transport_captures() {
    let mut input = input(
        "
        const Ctor = M;
        const captured = 'original-options';
        const mod = new Ctor(() => import('./target.mjs'), {label: captured});
        export function make(Ctor, transport, reader, extra) {
            return new R(mod, ns => ns.first, () => trace.push(Ctor), extra);
        }",
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    let (_, _, initial) = emit_lazy(&input, &[("entry.mjs", true)], false);
    let code = initial.files.values().cloned().collect::<String>();
    let options = code
        .split_once("var __gf_lazy_options_")
        .or_else(|| code.split_once("const __gf_lazy_options_"))
        .map(|(_, rest)| {
            format!(
                "__gf_lazy_options_{}",
                rest.split([' ', '=', ';']).next().unwrap()
            )
        })
        .unwrap();
    let reader = code
        .split_once("function __gf_lazy_constructor_")
        .map(|(_, rest)| format!("__gf_lazy_constructor_{}", rest.split('(').next().unwrap()))
        .unwrap();
    *input.get_mut(Path::new("entry.mjs")).unwrap() =
        input[Path::new("entry.mjs")].replace("transport, reader", &format!("{options}, {reader}"));
    for wire in [false, true] {
        let (graph, plan, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        assert_eq!(plan.lazy_modules.modules().len(), 1);
        let owner = |text| {
            &emitted.members[&crate::chunks::node_id(
                &graph,
                GraphNode::Segment(segment(&graph, &input, "entry.mjs", text)),
            )
            .unwrap()]
        };
        assert_eq!(owner("const Ctor"), owner("function make"));
        assert!(!emitted
            .files
            .values()
            .any(|text| text.contains("const mod")));
        observe_lazy(
            emitted,
            "
            const {make} = await load0();
            const selected = make(undefined, 'wrong-options', undefined, 'genuine-extra');
            assert.equal(selected.module.options.label, 'original-options');
            assert.equal(selected.extra, 'genuine-extra');
            assert.equal((await selected.getValue())(), 1);
            assert.deepEqual(trace, [undefined]);
        ",
        );
    }
}

#[test]
fn lazy_emission_relocated_wrappers_options_and_split_loaded_state() {
    let mut input = input(
        "
        const captured = 'capture';
        function optionEffect() { trace.push('options'); return captured; }
        trace.push('before');
        const mod = new M(() => import('./target.mjs'), {
            label: optionEffect(), onLoad: () => trace.push(captured)
        });
        export const a = new R(mod, ns => ns.first, () => trace.push('govern'), 'read-extra');
        export const b = new A(mod, ns => ns.second, {tag: 'action-extra'});
        const dead = new R(mod, ns => ns.unused);",
    );
    let selectors = input.remove(Path::new("entry.mjs")).unwrap();
    input.insert("selectors.mjs".into(), selectors);
    input.insert(
        "entry.mjs".into(),
        "import {a} from './selectors.mjs'; globalThis.a = a;".into(),
    );
    input.insert(
        "other.mjs".into(),
        "import {b} from './selectors.mjs'; globalThis.b = b;".into(),
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    input.insert(
        "target.mjs".into(),
        "import './effects.mjs'; export {first, second, unused} from './leaf.mjs';".into(),
    );
    input.insert("effects.mjs".into(), "trace.push('barrel');".into());
    input.insert("leaf.mjs".into(), "
        function shared() { return 4; }
        export function first() { return shared(); }
        export function second(value) { globalThis.attempts = (globalThis.attempts || 0) + 1; if (globalThis.attempts === 1) throw Error('retry'); return shared() + value; }
        export function unused() { return 'UNUSED_LAZY_IMPLEMENTATION'; }".into());
    for wire in [false, true] {
        let (graph, plan, emitted) =
            emit_lazy(&input, &[("entry.mjs", false), ("other.mjs", false)], wire);
        let owner = |path, text| {
            &emitted.members[&crate::chunks::node_id(
                &graph,
                GraphNode::Segment(segment(&graph, &input, path, text)),
            )
            .unwrap()]
        };
        assert_ne!(
            owner("leaf.mjs", "function first"),
            owner("leaf.mjs", "function second")
        );
        let code = emitted.files.values().cloned().collect::<String>();
        assert!(!code.contains("UNUSED_LAZY_IMPLEMENTATION"));
        assert!(!code.contains("const mod"));
        assert_eq!(plan.async_activations.len(), 2);
        observe_lazy(
            emitted,
            "
            await load0(); await load1();
            assert.deepEqual(trace, ['before','options']);
            assert.notEqual(a.module, b.module);
            assert.equal(a.module.options, b.module.options);
            assert.equal(a.module.options.label, 'capture');
            assert.equal(a.extra, 'read-extra');
            assert.equal(b.options.tag, 'action-extra');
            assert.equal(a.module.loaded, false);
            assert.equal(b.module.loaded, false);
            assert.equal(a.getter.name, '');
            const first = await a.getValue();
            assert.equal(first(), 4);
            assert.equal(a.calls, 1);
            assert.equal(a.module.loaded, true);
            assert.equal(b.module.loaded, false);
            assert.deepEqual(Object.keys(await a.module.getValue()), ['first']);
            await assert.rejects(b.invoke(3), /retry/);
            assert.equal(await b.invoke(3), 7);
            assert.equal(b.calls, 2);
            assert.equal(b.module.loaded, true);
            assert.deepEqual(trace, ['before','options','govern','barrel','capture','capture']);
        ",
        );
    }
}

#[test]
fn lazy_emission_transports_one_options_binding_to_distinct_selector_owners() {
    let mut input = input(
        "
        const captured = {label: 'shared-options'};
        const mod = new M(() => import('./target.mjs'), {captured});
        export const a = new R(mod, ns => ns.first);
        export const b = new A(mod, ns => ns.second);",
    );
    let selectors = input.remove(Path::new("entry.mjs")).unwrap();
    input.insert("selectors.mjs".into(), selectors);
    input.insert(
        "entry.mjs".into(),
        "import {a} from './selectors.mjs'; globalThis.a = a;".into(),
    );
    input.insert(
        "other.mjs".into(),
        "import {b} from './selectors.mjs'; globalThis.b = b;".into(),
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    let mut previous = None;
    for wire in [false, true] {
        let (graph, _, emitted) =
            emit_lazy(&input, &[("entry.mjs", false), ("other.mjs", false)], wire);
        if let Some(previous) = previous.replace(emitted.files.clone()) {
            assert_eq!(previous, emitted.files);
        }
        let owner = |text| {
            &emitted.members[&crate::chunks::node_id(
                &graph,
                GraphNode::Segment(segment(&graph, &input, "selectors.mjs", text)),
            )
            .unwrap()]
        };
        assert_ne!(owner("const a"), owner("const b"));
        let option_owner = owner("const mod");
        assert!(option_owner != owner("const a") || option_owner != owner("const b"));
        let code = emitted.files.values().cloned().collect::<String>();
        assert_eq!(code.matches("shared-options").count(), 1);
        assert!(!code.contains("const mod"));
        observe_lazy(
            emitted,
            "
            await load0(); await load1();
            assert.equal(a.module.options, b.module.options);
            assert.equal(a.module.options.captured.label, 'shared-options');
            assert.notEqual(a.module, b.module);
            assert.equal((await a.getValue())(), 1);
            assert.equal(b.module.loaded, false);
        ",
        );
    }
    let (_, _, emitted) = emit_lazy(
        &input,
        &[
            ("entry.mjs", false),
            ("other.mjs", false),
            ("selectors.mjs", true),
        ],
        true,
    );
    observe_lazy(
        emitted,
        "
        await load0(); await load1();
        assert.deepEqual(Object.keys(await load2()), ['a','b']);
        assert.equal(a.module.options, b.module.options);
    ",
    );
}

#[test]
fn lazy_certification_rejects_selector_construction_inside_its_module_initializer() {
    for options in [
        "{value: new R(mod, ns => ns.first)}",
        "{later: () => new R(mod, ns => ns.first)}",
    ] {
        let input = input(&format!("const mod = new M(() => import('./target.mjs'), {options}); export const a = new R(mod, ns => ns.first);"));
        for wire in [false, true] {
            let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
            assert!(certify(&graph, &EffectPolicy::default(), &[runtime()])
                .unwrap()
                .modules()
                .is_empty());
        }
    }
}

#[test]
fn lazy_nested_compound_initializers_without_exact_result_evidence_stay_original() {
    let input = input("
        const mod = new M(() => import('./target.mjs'));
        (() => { const selected = (trace.push('sequence'), new R(mod, ns => ns.first)); globalThis.selected = selected; })();");
    for wire in [false, true] {
        let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
        assert!(certify(&graph, &EffectPolicy::default(), &[runtime()])
            .unwrap()
            .modules()
            .is_empty());
    }
}

#[test]
fn lazy_exported_function_demand_does_not_keep_unused_nested_local_selectors() {
    let mut input = input(
        "
        const mod = new M(() => import('./target.mjs'));
        export function make() {
            const dead = new R(mod, ns => ns.second, trace.push('extra'));
            return new R(mod, ns => ns.first);
        }",
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    input.insert("leaf.mjs".into(), "export function first() { return 1; } export function second() { return 'DEAD_EXPORTED_FUNCTION_LOCAL'; }".into());
    for wire in [false, true] {
        let (_, plan, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        assert_eq!(plan.async_activations.len(), 1);
        assert!(!emitted
            .files
            .values()
            .any(|code| code.contains("DEAD_EXPORTED_FUNCTION_LOCAL")));
        observe_lazy(
            emitted,
            "
            const {make} = await load0();
            assert.deepEqual(trace, []);
            const selected = make();
            assert.deepEqual(trace, ['extra']);
            assert.equal((await selected.getValue())(), 1);
        ",
        );
    }
}

#[test]
fn lazy_external_native_linking_retains_the_whole_original_instance() {
    let mut input = input("import 'node:assert/strict'; const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    let graph = SourceGraph::from_owned(input.iter().map(|(path, source)| {
        let parsed =
            parse_module(&logger::StdioLogger::new(), source, path, ModuleFormat::Esm).unwrap();
        let resolutions = parsed
            .facts()
            .dependencies()
            .iter()
            .map(|dependency| {
                (
                    dependency.id(),
                    if dependency.specifier() == "node:assert/strict" {
                        DependencyTarget::External(dependency.specifier().into())
                    } else {
                        DependencyTarget::File(normalize(
                            &path.parent().unwrap().join(dependency.specifier()),
                        ))
                    },
                )
            })
            .collect();
        OwnedSourceFileInput::from_parsed(path.clone(), parsed, resolutions)
    }));
    let options = OutputOptions {
        lazy_runtimes: vec![runtime()],
        ..Default::default()
    };
    let plan = crate::chunks::partition_with_options(
        &graph,
        &roots(&graph),
        &EffectPolicy::default(),
        &options,
    )
    .unwrap();
    assert!(plan.lazy_modules.modules().is_empty());
    let emitted =
        esm_with_options(&graph, &plan, &EffectPolicy::default(), &input, &options).unwrap();
    observe_lazy(emitted, "const {a} = await load0(); assert.deepEqual(Object.keys(await a.module.getValue()), ['first','second']);");
    let mut invalid = options;
    invalid.lazy_runtimes[0].module.export = "Missing".into();
    assert!(crate::chunks::partition_with_options(
        &graph,
        &roots(&graph),
        &EffectPolicy::default(),
        &invalid,
    )
    .unwrap_err()
    .to_string()
    .contains("invalid lazy runtime constructor identity"));
}

#[test]
fn lazy_emission_shares_only_matching_owner_module_and_destination_with_collisions() {
    let mut input = input(
        "
        const __gf_lazy_0 = 'collision';
        const ModuleAlias = M; const ReadAlias = R;
        const mod = new ModuleAlias(() => import('./target.mjs'));
        const other = new M(() => import('./target.mjs'));
        export const a = new ReadAlias(mod, ns => ns.first);
        export const b = new R(mod, ns => ns.first);
        export const c = new A(mod, ns => ns.second);
        export const d = new R(other, ns => ns.first);
        export const collision = __gf_lazy_0;",
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    let (_, _, initial) = emit_lazy(&input, &[("entry.mjs", true)], false);
    let generated = initial
        .files
        .values()
        .find_map(|source| {
            source
                .split_once("var __gf_lazy_")
                .map(|(_, rest)| format!("__gf_lazy_{}", rest.split_once(';').unwrap().0))
        })
        .unwrap();
    *input.get_mut(Path::new("entry.mjs")).unwrap() =
        input[Path::new("entry.mjs")].replace("__gf_lazy_0", &generated);
    for wire in [false, true] {
        let (_, plan, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        assert_eq!(plan.lazy_modules.modules().len(), 2);
        observe_lazy(
            emitted,
            "
            const {a,b,c,d,collision} = await load0();
            assert.equal(collision, 'collision');
            assert.equal(a.module, b.module);
            assert.notEqual(a.module, c.module);
            assert.notEqual(a.module, d.module);
            assert.equal((await b.getValue())(), 1);
            assert.equal(a.module.loaded, true);
            assert.equal(c.module.loaded, false);
            assert.equal(d.module.loaded, false);
            assert.deepEqual(Object.keys(await a.module.getValue()), ['first']);
        ",
        );
    }
}

#[test]
fn lazy_emission_nested_captures_preceding_initializers_prune_unrelated_local_reads() {
    let mut input = input("
        const mod = new M(() => import('./target.mjs'));
        (() => { globalThis.getSelected = () => selected; const selected = new R(mod, ns => ns.first); })();
        (() => { const token = 1; const dead = new R(mod, ns => ns.second, trace.push('dead-extra')); globalThis.token = token; })();");
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    input.insert("leaf.mjs".into(), "export function first() { return 'retained'; } export function second() { return 'DEAD_NESTED_IMPLEMENTATION'; }".into());
    for wire in [false, true] {
        let (_, plan, emitted) = emit_lazy(&input, &[("entry.mjs", false)], wire);
        assert_eq!(plan.async_activations.len(), 1);
        assert!(
            !emitted
                .files
                .values()
                .any(|code| code.contains("DEAD_NESTED_IMPLEMENTATION")
                    || code.contains("ns.second")),
            "{:#?}",
            emitted.files
        );
        observe_lazy(
            emitted,
            "
            await load0();
            assert.equal(token, 1);
            assert.deepEqual(trace, ['dead-extra']);
            const selected = getSelected();
            assert.equal(selected.module.loaded, false);
            assert.equal((await selected.getValue())(), 'retained');
        ",
        );
    }
}

#[test]
fn lazy_emission_nested_conditional_wrappers_keep_construction_and_options_timing() {
    let mut input = input("
        trace.push('before');
        const captured = 'captured';
        function option() { trace.push(captured); return captured; }
        const mod = new M(() => import('./target.mjs'), {name: option()});
        trace.push('after');
        export function make(flag) { return flag ? new R(mod, ns => ns.first, undefined, extra()) : undefined; }
        function extra() { trace.push('extra'); return 9; }");
    input.insert(
        "runtime.mjs".into(),
        BEHAVIOR_RUNTIME.replace(
            "this.extra = extra;",
            "trace.push('read:new'); this.extra = extra;",
        ),
    );
    for wire in [false, true] {
        let (_, _, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        observe_lazy(
            emitted,
            "
            const {make} = await load0();
            assert.deepEqual(trace, ['before','captured','after']);
            assert.equal(make(false), undefined);
            assert.deepEqual(trace, ['before','captured','after']);
            const value = make(true);
            assert.equal(value.extra, 9);
            assert.deepEqual(trace, ['before','captured','after','extra','read:new']);
            assert.equal(value.module.options.name, 'captured');
            assert.equal((await value.getValue())(), 1);
        ",
        );
    }
}

#[test]
fn lazy_emission_keeps_original_barrel_tla_effects_and_cached_failure() {
    let mut input = input(
        "
        const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);
        export const b = new R(mod, ns => ns.second);",
    );
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    input.insert(
        "target.mjs".into(),
        "
        trace.push('start'); await Promise.resolve(); trace.push('done');
        if (globalThis.fail) throw globalThis.failure;
        export {first,second} from './leaf.mjs';"
            .into(),
    );
    for wire in [false, true] {
        let (_, _, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        observe_lazy(
            emitted.clone(),
            "
            const {a,b} = await load0();
            assert.deepEqual(trace, []);
            assert.equal(a.module, b.module);
            const result = a.getValue();
            assert.equal(a.module.loaded, false);
            assert.equal((await result)(), 1);
            assert.equal((await b.getValue())(), 2);
            assert.deepEqual(Object.keys(await a.module.getValue()), ['first','second']);
            assert.deepEqual(trace, ['start','done']);
        ",
        );
        observe_lazy(
            emitted,
            "
            globalThis.fail = true; globalThis.failure = {message: 'barrel failure'};
            const {a,b} = await load0();
            for (const selector of [a,a,b]) {
                await assert.rejects(selector.getValue(), error => error === failure);
                assert.equal(selector.module.loaded, false);
            }
            assert.deepEqual(trace, ['start','done']);
            assert.equal(a.calls, 2);
        ",
        );
    }
}

#[test]
fn lazy_emission_tla_selector_source_and_options_only_residual_use_checked_recipes() {
    for body in [
        "await Promise.resolve();
        const mod = new M(() => import('./target.mjs'));
        (() => { globalThis.selected = new R(mod, ns => ns.first); })();",
        "await Promise.resolve();
        function option() { trace.push('option-only'); return 1; }
        const mod = new M(() => import('./target.mjs'), {name: option()});
        const dead = new R(mod, ns => ns.second);",
    ] {
        let mut input = input(body);
        input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
        for wire in [false, true] {
            let (_, plan, emitted) = emit_lazy(&input, &[("entry.mjs", false)], wire);
            assert!(!emitted
                .files
                .values()
                .any(|code| code.contains("const mod")));
            if plan.async_activations.is_empty() {
                observe_lazy(emitted, "await load0(); assert.deepEqual(trace, ['option-only']); assert.equal(globalThis.barrelEffect, undefined);");
            } else {
                observe_lazy(emitted, "await load0(); assert.equal(selected.module.loaded, false); assert.equal((await selected.getValue())(), 1);");
            }
        }
    }
}

#[test]
fn lazy_emission_mixed_initializer_escape_and_shadow_instances_stay_original() {
    let mut input = input("
        const broad = new M(() => import('./target.mjs'), {initializer: () => trace.push('initializer')});
        export const original = new R(broad, ns => ns.first);
        globalThis.escape = broad;
        const mod = new M(() => import('./target.mjs'));
        export const selected = new R(mod, ns => ns.first);
        export function shadow(M, R) { const mod = new M(() => import('./target.mjs')); return new R(mod, ns => ns.first); }");
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    for wire in [false, true] {
        let (_, plan, emitted) = emit_lazy(&input, &[("entry.mjs", true)], wire);
        assert_eq!(plan.lazy_modules.modules().len(), 1);
        observe_lazy(
            emitted,
            "
            const {original,selected,shadow} = await load0();
            assert.equal(original.module, escape);
            assert.notEqual(selected.module, escape);
            assert.equal((await original.getValue())(), 1);
            assert.deepEqual(Object.keys(await escape.getValue()), ['first','second']);
            assert.deepEqual(trace, ['initializer']);
            class M { constructor(load) { this.load = load; } }
            class R { constructor(mod, get) { this.mod = mod; this.get = get; } }
            const value = shadow(M,R);
            assert.equal(value.mod instanceof M, true);
            assert.equal(value.get(await value.mod.load())(), 1);
        ",
        );
    }
}

#[test]
fn lazy_certification_accepts_registered_immutable_default_constructor_expressions() {
    let mut input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
    input.insert(
        "runtime.mjs".into(),
        RUNTIME.replace("export class ModuleCtor", "export default class"),
    );
    input.insert("runtime-barrel.mjs".into(), "export {default as Module, ImportCtor as Read, ActionCtor as Action} from './runtime.mjs';".into());
    let mut runtime = runtime();
    runtime.module.export = "default".into();
    for wire in [false, true] {
        let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
        let checked = certify(&graph, &EffectPolicy::default(), &[runtime.clone()]).unwrap();
        assert_eq!(checked.modules().len(), 1);
        assert!(matches!(
            checked.modules()[0].constructor,
            source_graph::ExportIdentity::Expression(_)
        ));
    }
}

fn runtime() -> LazyRuntime {
    let export = |name: &str| ConstructorExport {
        module: "runtime.mjs".into(),
        export: name.into(),
    };
    LazyRuntime {
        module: export("ModuleCtor"),
        import: export("ImportCtor"),
        action: export("ActionCtor"),
    }
}

fn input(body: &str) -> BTreeMap<PathBuf, String> {
    sources(&[
        ("entry.mjs", &format!("{IMPORTS}\n{body}")),
        ("runtime.mjs", RUNTIME),
        ("runtime-barrel.mjs", "export {ModuleCtor as Module, ImportCtor as Read, ActionCtor as Action} from './runtime.mjs';"),
        ("target.mjs", "import './effects.mjs'; export {first, second} from './leaf.mjs';"),
        ("leaf.mjs", "export function first() { return 1; } export function second() { return 2; } export function unused() { return 3; }"),
        ("effects.mjs", "globalThis.barrelEffect = true;"),
    ])
}

fn roots(graph: &SourceGraph) -> Vec<RootSeed> {
    vec![RootSeed {
        id: RootId::Entry("entry".into()),
        evaluation_roots: vec![graph.file_id(Path::new("entry.mjs")).unwrap()],
        binding_roots: vec![GraphNode::Module(
            graph.file_id(Path::new("entry.mjs")).unwrap(),
        )],
    }]
}

fn planned(input: &BTreeMap<PathBuf, String>, wire: bool) -> (SourceGraph, ChunkPlan) {
    let graph = graph_with_storage(input, wire, ModuleFormat::Esm, wire);
    let plan = crate::chunks::partition_with_options(
        &graph,
        &roots(&graph),
        &EffectPolicy::default(),
        &OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        },
    )
    .unwrap();
    (graph, plan)
}

fn segment(
    graph: &SourceGraph,
    input: &BTreeMap<PathBuf, String>,
    path: &str,
    text: &str,
) -> source_graph::SegmentKey {
    let file = graph.file_id(Path::new(path)).unwrap();
    let index = graph
        .file_segments(file)
        .unwrap()
        .iter()
        .position(|segment| {
            input[Path::new(path)][segment.span.start as usize..segment.span.end as usize]
                .contains(text)
        })
        .unwrap();
    source_graph::SegmentKey::new(file, index as u32)
}

fn mask<'a>(
    graph: &SourceGraph,
    plan: &'a ChunkPlan,
    key: source_graph::SegmentKey,
) -> &'a BTreeSet<RootId> {
    &plan.node_roots[&crate::chunks::node_id(graph, GraphNode::Segment(key)).unwrap()]
}

#[test]
fn lazy_native_placement_separates_selectors_and_their_selected_implementations() {
    let mut input = input(
        "const mod = new M(() => import('./target.mjs'));
        function captured() { return 7; }
        export const a = new R(mod, ns => ns.first, () => captured());
        export const b = new A(mod, ns => ns.second, {other: () => captured()});",
    );
    let selectors = input.remove(Path::new("entry.mjs")).unwrap();
    input.insert("selectors.mjs".into(), selectors);
    input.insert(
        "entry.mjs".into(),
        "import {a} from './selectors.mjs'; globalThis.a = a;".into(),
    );
    input.insert(
        "other.mjs".into(),
        "import {b} from './selectors.mjs'; globalThis.b = b;".into(),
    );
    input.insert("leaf.mjs".into(),
        "function shared() { return 1; } export function first() { return shared(); } export function second() { return shared() + 1; }".into());
    input.insert("runtime.mjs".into(), BEHAVIOR_RUNTIME.into());
    for wire in [false, true] {
        let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
        let options = OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        };
        let roots: Vec<_> = ["entry.mjs", "other.mjs"]
            .into_iter()
            .map(|name| RootSeed {
                id: RootId::Entry(name.into()),
                evaluation_roots: vec![graph.file_id(Path::new(name)).unwrap()],
                binding_roots: vec![],
            })
            .collect();
        let plan = crate::chunks::partition_with_options(
            &graph,
            &roots,
            &EffectPolicy::default(),
            &options,
        )
        .unwrap();
        let parses = ast_segmenter::module_parse_count();
        let placement =
            native_placement(&graph, &plan, &EffectPolicy::default(), &input, &options).unwrap();
        assert_eq!(ast_segmenter::module_parse_count(), parses);
        let a = segment(&graph, &input, "selectors.mjs", "const a");
        let b = segment(&graph, &input, "selectors.mjs", "const b");
        assert_ne!(placement.owners()[&a], placement.owners()[&b]);
        assert_eq!(
            placement.owners()[&a],
            NativeOwner::Evaluation(graph.file_id(Path::new("entry.mjs")).unwrap())
        );
        assert_eq!(
            placement.owners()[&b],
            NativeOwner::Evaluation(graph.file_id(Path::new("other.mjs")).unwrap())
        );
        let first = segment(&graph, &input, "leaf.mjs", "function first");
        let second = segment(&graph, &input, "leaf.mjs", "function second");
        assert_ne!(placement.owners()[&first], placement.owners()[&second]);
        assert_eq!(
            mask(&graph, &plan, a),
            &BTreeSet::from([RootId::Entry("entry.mjs".into())])
        );
        assert_eq!(
            mask(&graph, &plan, b),
            &BTreeSet::from([RootId::Entry("other.mjs".into())])
        );
        assert_eq!(placement.lazy().len(), 2);
        let module = &plan.lazy_modules.modules()[0];
        assert!(!placement.owners().contains_key(&module.owner));
        assert!(plan
            .retention
            .edges
            .iter()
            .all(|edge| edge.to != GraphNode::Segment(module.owner)));
        for wiring in placement.lazy() {
            assert_eq!(wiring.source, placement.owners()[&wiring.selector_segment]);
            assert_eq!(
                &wiring.roots,
                mask(
                    &graph,
                    &plan,
                    if wiring.selector_segment == a {
                        first
                    } else {
                        second
                    }
                )
            );
            assert_eq!(
                wiring.destination,
                placement.owners()[&if wiring.selector_segment == a {
                    first
                } else {
                    second
                }]
            );
            assert_eq!(
                wiring.evaluation_target,
                graph.file_id(Path::new("target.mjs")).unwrap()
            );
            assert!(wiring.options.is_none());
            assert!(placement.bindings()[&wiring.source].iter().any(|binding| {
                binding.symbol == wiring.constructor_symbol
                    && binding.identity == wiring.constructor
                    && binding.owner == Some(wiring.constructor_owner)
            }));
        }
        let union: BTreeSet<_> = mask(&graph, &plan, first)
            .union(mask(&graph, &plan, second))
            .cloned()
            .collect();
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "leaf.mjs", "function shared")
            ),
            union
        );
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "effects.mjs", "barrelEffect")
            ),
            union
        );
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "selectors.mjs", "function captured")
            ),
            BTreeSet::from([
                RootId::Entry("entry.mjs".into()),
                RootId::Entry("other.mjs".into())
            ])
        );
        let emitted =
            esm_with_options(&graph, &plan, &EffectPolicy::default(), &input, &options).unwrap();
        let owner = |key: source_graph::SegmentKey| {
            &emitted.members[&crate::chunks::node_id(&graph, GraphNode::Segment(key)).unwrap()]
        };
        assert_ne!(owner(a), owner(b));
        assert_ne!(owner(first), owner(second));
        let shared = segment(&graph, &input, "leaf.mjs", "function shared");
        let effects = segment(&graph, &input, "effects.mjs", "barrelEffect");
        assert_ne!(owner(shared), owner(first));
        assert_ne!(owner(shared), owner(second));
        assert_ne!(owner(effects), owner(first));
        assert_ne!(owner(effects), owner(second));
        assert!(!emitted.members.contains_key(
            &crate::chunks::node_id(&graph, GraphNode::Segment(module.owner)).unwrap()
        ));
        observe_lazy(
            emitted,
            "
            await load0(); await load1();
            assert.notEqual(a.module, b.module);
            assert.equal(globalThis.barrelEffect, undefined);
            assert.equal((await a.getValue())(), 1);
            assert.equal(globalThis.barrelEffect, true);
            assert.equal(b.module.loaded, false);
            assert.equal(await b.invoke(), 2);
        ",
        );
    }
}

#[test]
fn lazy_effect_only_options_do_not_retain_the_removed_constructor_reference() {
    let input = input(
        "function record() { globalThis.optionEffect = true; return 'name'; }
        globalThis.runtime = R;
        const mod = new M(() => import('./target.mjs'), {name: record()});
        const dead = new R(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.lazy_modules.modules().len(), 1);
        assert!(plan.async_activations.is_empty());
        let options = OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        };
        let placement =
            native_placement(&graph, &plan, &EffectPolicy::default(), &input, &options).unwrap();
        assert!(placement.lazy().is_empty());
        let constructor = segment(&graph, &input, "runtime.mjs", "class ModuleCtor");
        assert!(mask(&graph, &plan, constructor).is_empty());
        assert!(!placement.owners().contains_key(&constructor));
    }
}

#[test]
fn lazy_native_placement_keeps_one_options_value_and_original_effect_fences() {
    for options_value in [
        "{other: () => captured}",
        "{name: record(), other: () => captured}",
    ] {
        let input = input(&format!(
            "const captured = {{}};
            function record() {{ globalThis.optionsEffect = true; return captured; }}
            globalThis.beforeOptions = true;
            const mod = new M(() => import('./target.mjs'), {options_value});
            export const a = new R(mod, ns => ns.first);
            export const b = new A(mod, ns => ns.second);
            globalThis.afterOptions = true;"
        ));
        for wire in [false, true] {
            let (graph, plan) = planned(&input, wire);
            let options = OutputOptions {
                lazy_runtimes: vec![runtime()],
                ..Default::default()
            };
            let placement =
                native_placement(&graph, &plan, &EffectPolicy::default(), &input, &options)
                    .unwrap();
            let module = &plan.lazy_modules.modules()[0];
            let value = placement.lazy()[0].options.as_ref().unwrap();
            assert_eq!(value.segment, module.owner);
            assert_eq!(value.span, module.options.as_ref().unwrap().span);
            assert!(placement
                .lazy()
                .iter()
                .all(|wiring| wiring.options.as_ref() == Some(value)));
            let schedule = placement.schedule(value.owner);
            assert_eq!(
                schedule.iter().filter(|&&key| key == module.owner).count(),
                1
            );
            let before = segment(&graph, &input, "entry.mjs", "beforeOptions");
            let after = segment(&graph, &input, "entry.mjs", "afterOptions");
            assert_eq!(placement.owners()[&before], value.owner);
            assert_eq!(placement.owners()[&after], value.owner);
            let position = |key| {
                schedule
                    .iter()
                    .position(|&scheduled| scheduled == key)
                    .unwrap()
            };
            assert!(position(before) < position(module.owner));
            assert!(position(module.owner) < position(after));
            assert!(plan
                .retention
                .execution
                .constraints
                .iter()
                .any(|constraint| {
                    constraint.before == before
                        && constraint.after == module.owner
                        && constraint.kind
                            == crate::execution_dependencies::ConstraintKind::AbruptCompletion
                }));
            assert!(!plan.retention.execution.accesses.iter().any(|access| {
                access.segment == module.owner && access.reference == module.constructor_reference
            }));
            assert!(placement
                .bindings()
                .values()
                .flatten()
                .all(|binding| binding.symbol != (module.owner.file_id, module.binding)));
            assert!(!mask(
                &graph,
                &plan,
                segment(&graph, &input, "entry.mjs", "const captured")
            )
            .is_empty());
        }
    }
}

#[test]
fn lazy_overlays_replace_broad_demand_with_independent_selector_roots() {
    let mut input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);
        export const b = new R(mod, ns => ns.second);
        const unused = new A(mod, ns => ns.unused);",
    );
    input.insert(
        "target.mjs".into(),
        "import './effects.mjs'; export * from './leaf.mjs';".into(),
    );
    input.insert(
        "leaf.mjs".into(),
        "function shared() { return 1; }
        export function first() { return shared(); }
        export function second() { return shared() + 1; }
        export function unused() { return import('./dead.mjs'); }"
            .into(),
    );
    input.insert("dead.mjs".into(), "throw new Error('dead');".into());
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert!(mask(&graph, &plan, module.owner).is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const unused")
        )
        .is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function unused")
        )
        .is_empty());
        assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
        assert!(!plan.retention.edges.iter().any(|edge| {
            edge.source_edge == Some(module.source_edge)
                || edge.to == GraphNode::Segment(module.owner)
        }));
        let a = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first"),
        );
        let b = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function second"),
        );
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert!(a.is_disjoint(b));
        let union: BTreeSet<_> = a.union(b).cloned().collect();
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "leaf.mjs", "function shared")
            ),
            union
        );
        assert_eq!(
            *mask(
                &graph,
                &plan,
                segment(&graph, &input, "effects.mjs", "barrelEffect")
            ),
            union
        );
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "runtime.mjs", "class ModuleCtor")
        )
        .is_empty());
        let manifest = diagnostic_manifest(&graph, &plan, &EffectPolicy::default()).unwrap();
        for (site, activation) in &plan.async_activations {
            let crate::chunks::AsyncOrigin::Lazy {
                declaration,
                module_candidate,
                selector,
            } = activation.origin
            else {
                panic!("no original import activation");
            };
            assert_eq!(declaration, module.owner);
            assert_eq!(module_candidate, module.candidate);
            let selected = module
                .selectors
                .iter()
                .find(|s| s.candidate == selector)
                .unwrap();
            assert_eq!(site.offset, selected.construction.start);
            let facade = &manifest.async_facades[site];
            assert_eq!(facade.target, NodeId::Module("target.mjs".into()));
            assert_eq!(
                facade.consumer,
                crate::chunks::node_id(&graph, GraphNode::Segment(selected.owner)).unwrap()
            );
            assert_eq!(facade.demand_roots.len(), 1);
        }
    }
}

#[test]
fn lazy_residual_options_and_unused_wrapper_arguments_preserve_real_dependencies() {
    let input = input("const capture = {value: 7};
        function helper() { return capture; }
        function record(value) { globalThis.seen = value; return value; }
        const mod = new M(() => import('./target.mjs'), {name: record(capture), other: () => helper()});
        export const a = new R(mod, ns => ns.first, () => helper());
        export const b = new R(mod, ns => ns.first);
        const unused = new A(mod, ns => ns.second, record(capture));
        const dead = new R(mod, ns => ns.second);");
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert!(!plan
            .retention
            .edges
            .iter()
            .any(|edge| edge.kind == crate::RetentionEdgeKind::Binding
                && edge.to == GraphNode::Segment(module.owner)));
        assert!(plan
            .retention
            .edges
            .iter()
            .any(|edge| edge.kind == crate::RetentionEdgeKind::PreservedSlot
                && edge.to == GraphNode::Segment(module.owner)));
        let original = plan
            .retention
            .segments
            .iter()
            .find(|s| s.key == module.owner)
            .unwrap();
        assert!(original
            .reasons
            .iter()
            .all(|reason| !reason.demands_value()));
        let symbols = crate::symbols::Symbols::link(&graph, &plan).unwrap();
        assert!(!symbols
            .definitions
            .contains_key(&(module.owner.file_id, module.binding)));
        assert!(!plan
            .retention
            .execution
            .accesses
            .iter()
            .any(|access| access.declaration == module.owner && access.segment != module.owner));
        assert!(!plan
            .retention
            .execution
            .constraints
            .iter()
            .any(|constraint| constraint.before == module.owner
                && module
                    .selectors
                    .iter()
                    .any(|selector| selector.owner == constraint.after)
                && matches!(
                    constraint.kind,
                    crate::execution_dependencies::ConstraintKind::Initialization
                        | crate::execution_dependencies::ConstraintKind::ReadWrite
                )));
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == module.owner)
            .unwrap()
            .summary;
        assert_eq!(
            summary
                .rewrites
                .iter()
                .filter(|r| r.invocation == module.construction)
                .count(),
            1
        );
        assert!(!summary
            .sites
            .iter()
            .any(|site| site.span() == module.construction));
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
        for text in [
            "const capture",
            "function helper",
            "function record",
            "const unused",
        ] {
            assert!(
                !mask(&graph, &plan, segment(&graph, &input, "entry.mjs", text)).is_empty(),
                "{text}"
            );
        }
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const dead")
        )
        .is_empty());
        let unused = segment(&graph, &input, "entry.mjs", "const unused");
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == unused)
            .unwrap()
            .summary;
        assert!(summary.is_effectful());
        assert_eq!(summary.rewrites.len(), 1);
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
    }
}

#[test]
fn lazy_pure_options_capture_is_retained_at_original_slot_without_module_value() {
    let input = input(
        "const captured = {};
        const mod = new M(() => import('./target.mjs'), {other: () => captured});
        export const a = new R(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        let retained = plan
            .retention
            .segments
            .iter()
            .find(|s| s.key == module.owner)
            .unwrap();
        assert_eq!(retained.reasons, [crate::RetentionReason::PreservedSlot]);
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "const captured")
        )
        .is_empty());
    }
}

#[test]
fn lazy_selected_demands_discover_nested_instances_but_not_dead_ones() {
    let mut input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);",
    );
    input.insert(
        "leaf.mjs".into(),
        "import {next} from './nested.mjs';
        export function first() { return next; } export function second() { return 2; }"
            .into(),
    );
    input.insert(
        "nested.mjs".into(),
        format!(
            "{IMPORTS}
        const nested = new M(() => import('./last.mjs'));
        export const next = new R(nested, ns => ns.value);
        const dead = new M(() => import('./dead.mjs'));
        const ignored = new R(dead, ns => ns.value);"
        ),
    );
    input.insert(
        "last.mjs".into(),
        "export const value = 3; export const sibling = 4;".into(),
    );
    input.insert("dead.mjs".into(), "export const value = 5;".into());
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.lazy_modules.modules().len(), 3);
        assert_eq!(plan.async_activations.len(), 2);
        assert!(plan.retention.async_boundaries.is_empty());
        assert_eq!(
            mask(
                &graph,
                &plan,
                segment(&graph, &input, "last.mjs", "const value")
            )
            .len(),
            1
        );
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "last.mjs", "const sibling")
        )
        .is_empty());
        assert!(plan.node_roots[&NodeId::Module("dead.mjs".into())].is_empty());
    }
}

#[test]
fn lazy_nested_escaping_result_is_not_confused_with_getter_local_reads() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        (() => { globalThis.getSelected = () => selected; const selected = new R(mod, ns => ns.first); })();
        (() => { const token = 1; const dead = new R(mod, ns => ns.second); globalThis.token = token; })();",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        assert_eq!(module.selectors.len(), 2);
        assert!(module.selectors[0].nested_result_read);
        assert!(!module.selectors[1].nested_result_read);
        assert_eq!(plan.async_activations.len(), 1);
        let first = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first"),
        );
        assert_eq!(first.len(), 1);
        assert!(first
            .iter()
            .all(|root| matches!(root, RootId::AsyncExport { name, .. } if name == "first")));
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function second")
        )
        .is_empty());
        for selector in &module.selectors {
            let summary = &plan
                .retention
                .effects
                .iter()
                .find(|effect| effect.key == selector.owner)
                .unwrap()
                .summary;
            assert_eq!(
                summary
                    .rewrites
                    .iter()
                    .any(|rewrite| rewrite.invocation == selector.construction),
                !selector.nested_result_read,
            );
        }
    }
}

#[test]
fn lazy_required_wrapper_values_activate_without_statement_binding_demand() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        globalThis.used = new R(mod, ns => ns.first);
        new A(mod, ns => ns.second);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.async_activations.len(), 1);
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first")
        )
        .is_empty());
        assert!(mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function second")
        )
        .is_empty());
        assert!(mask(&graph, &plan, segment(&graph, &input, "entry.mjs", "new A")).is_empty());
    }
}

#[test]
fn lazy_invocation_overlay_preserves_other_expressions_in_the_same_statement() {
    let input = input(
        "function effect() { globalThis.seen = true; }
        const mod = new M(() => import('./target.mjs'));
        const unused = (new A(mod, ns => ns.second), effect());
        export const a = new R(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        let module = &plan.lazy_modules.modules()[0];
        let selector = &module.selectors[0];
        let summary = &plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == selector.owner)
            .unwrap()
            .summary;
        assert!(summary.is_effectful());
        assert!(summary
            .rewrites
            .iter()
            .any(|r| r.invocation == selector.construction));
        assert!(!summary
            .sites
            .iter()
            .any(|s| s.span() == selector.construction));
        assert_eq!(
            summary
                .sites
                .iter()
                .filter(|site| site.reason() == ast_segmenter::effects::EffectReason::Invocation)
                .count(),
            1
        );
        assert!(!mask(
            &graph,
            &plan,
            segment(&graph, &input, "entry.mjs", "function effect")
        )
        .is_empty());
        assert!(plan.retention.async_boundaries.is_empty());
        assert_eq!(plan.async_activations.len(), 1);
    }
}

#[test]
fn lazy_same_export_selectors_keep_distinct_activation_and_root_identities() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first);
        export const b = new A(mod, ns => ns.first);",
    );
    for wire in [false, true] {
        let (graph, plan) = planned(&input, wire);
        assert_eq!(plan.async_activations.len(), 2);
        let selected = mask(
            &graph,
            &plan,
            segment(&graph, &input, "leaf.mjs", "function first"),
        );
        assert_eq!(selected.len(), 2);
        assert!(selected
            .iter()
            .all(|root| matches!(root, RootId::AsyncExport { name, .. } if name == "first")));
        assert!(plan
            .async_activations
            .values()
            .all(|activation| activation.roots.len() == 1));
    }
}

#[test]
fn lazy_mixed_instance_retains_the_original_broad_graph_unchanged() {
    let input = input(
        "const mod = new M(() => import('./target.mjs'));
        export const a = new R(mod, ns => ns.first); globalThis.escape = mod;",
    );
    for wire in [false, true] {
        let (graph, enabled) = planned(&input, wire);
        let disabled =
            crate::chunks::partition(&graph, &roots(&graph), &EffectPolicy::default()).unwrap();
        assert!(enabled.lazy_modules.modules().is_empty());
        assert_eq!(enabled.retention, disabled.retention);
        assert_eq!(enabled.roots, disabled.roots);
        assert_eq!(enabled.async_activations, disabled.async_activations);
        assert!(!mask(
            &graph,
            &enabled,
            segment(&graph, &input, "leaf.mjs", "function second")
        )
        .is_empty());
    }
}

#[test]
fn lazy_certification_uses_canonical_aliased_barrel_identities_for_overlays() {
    let input = input("const ModuleAlias = M, ReadAlias = R;
        const captured = {value: 1};
        function effect(value) { globalThis.seen = value; return value; }
        const mod = new ModuleAlias(() => import('./target.mjs'), { name: effect(captured), captured });
        export const a = new ReadAlias(mod, ns => ns.first, effect(captured)),
            b = new A(mod, ns => ns.second, {bundleTime: effect(captured)});
        function shadow(M, R) { const mod = new M(() => import('./target.mjs')); return new R(mod, ns => ns.first); }");
    for wire in [false, true] {
        let graph = graph_with_storage(&input, wire, ModuleFormat::Esm, wire);
        let policy = EffectPolicy::default();
        let disabled = crate::chunks::partition(&graph, &roots(&graph), &policy).unwrap();
        assert!(disabled.lazy_modules.modules().is_empty());
        let options = OutputOptions {
            lazy_runtimes: vec![runtime()],
            ..Default::default()
        };
        let parses = ast_segmenter::module_parse_count();
        let plan = crate::chunks::partition_with_options(&graph, &roots(&graph), &policy, &options)
            .unwrap();
        assert_eq!(ast_segmenter::module_parse_count(), parses);
        assert!(plan.is_current_with_options(&graph, &policy, &options));
        assert!(!plan.is_current_with_options(&graph, &policy, &OutputOptions::default()));
        assert_ne!(plan.retention, disabled.retention);
        assert_ne!(plan.roots, disabled.roots);
        let [module] = plan.lazy_modules.modules() else {
            panic!("one top-level module")
        };
        assert_eq!(
            module.target,
            graph.file_id(Path::new("target.mjs")).unwrap()
        );
        assert_eq!(
            graph.edges()[module.source_edge].dependency,
            Some(module.dependency)
        );
        assert_eq!(module.selectors.len(), 2);
        assert_eq!(module.selectors[0].kind, LazyWrapperKind::LazyImport);
        assert_eq!(module.selectors[1].kind, LazyWrapperKind::LazyAction);
        assert!(module.selectors.iter().all(|s| matches!(s.selected.identity,
            source_graph::ExportIdentity::Binding(file, _) if Some(file) == graph.file_id(Path::new("leaf.mjs")))));
        let preserved = module.options.as_ref().unwrap();
        assert!(!preserved.operations.is_empty());
        assert!(!preserved.effects.is_empty());
        assert_eq!(preserved.invocations.len(), 1);
        let facts = graph.file_facts(module.owner.file_id).unwrap();
        assert!(preserved.references.iter().any(|&r| facts
            .references()
            .get(r as usize)
            .unwrap()
            .name()
            == "captured"));
        assert_eq!(module.selectors[0].arguments.len(), 3);
        assert_eq!(module.selectors[0].arguments[2].invocations.len(), 1);
        assert_eq!(module.selectors[1].arguments[2].invocations.len(), 1);
        esm(&graph, &disabled, &policy, &input).unwrap();
        esm_with_options(&graph, &plan, &policy, &input, &options).unwrap();
        assert!(esm(&graph, &plan, &policy, &input).is_err());
        assert!(deferred_esm(&graph, &plan, &policy, &input)
            .unwrap()
            .is_none());
        assert!(
            esm_with_options(&graph, &plan, &policy, &input, &OutputOptions::default()).is_err()
        );
        let changed_policy = EffectPolicy::default().with_pure_annotations(false);
        assert!(!plan
            .lazy_modules
            .is_current(&graph, &changed_policy, &options.lazy_runtimes));
        let mut other_graph = graph_with_storage(&input, !wire, ModuleFormat::Esm, wire);
        assert!(other_graph.remove_file(Path::new("effects.mjs")));
        assert!(!plan
            .lazy_modules
            .is_current(&other_graph, &policy, &options.lazy_runtimes));
    }
}

#[test]
fn lazy_certification_falls_back_for_the_entire_mixed_or_escaped_instance() {
    for extra in [
        "export {mod};",
        "globalThis.escape = mod;",
        "mod.getValue();",
        "mod.preload();",
        "unknown(mod);",
        "const alias = mod; const b = new R(alias, ns => ns.second);",
        "function neverCalled() { unknown(mod); }",
        "const Other = class {}; const b = new Other(mod, ns => ns.second);",
        "const b = new R(mod, ns => ns.second());",
        "const b = new R(mod, ns => ns['second']);",
        "const b = new A(mod, ns => ns.second, ...unknown);",
    ] {
        let input = input(&format!(
            "const mod = new M(() => import('./target.mjs'));
            export const a = new R(mod, ns => ns.first); {extra}
            const supported = new M(() => import('./target.mjs'));
            export const independent = new R(supported, ns => ns.second);"
        ));
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            let checked = certify(&graph, &EffectPolicy::default(), &[runtime()]).unwrap();
            assert_eq!(checked.modules().len(), 1, "{extra}");
            let facts = graph
                .file_facts(checked.modules()[0].owner.file_id)
                .unwrap();
            assert_eq!(
                facts
                    .bindings()
                    .iter()
                    .find(|b| b.id() == checked.modules()[0].binding)
                    .unwrap()
                    .name(),
                "supported",
                "{extra}"
            );
            let plan = crate::chunks::partition_with_options(
                &graph,
                &roots(&graph),
                &EffectPolicy::default(),
                &OutputOptions {
                    lazy_runtimes: vec![runtime()],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(plan.retention.async_boundaries.len(), 1, "{extra}");
            assert!(
                !plan
                    .retention
                    .async_boundaries
                    .contains(&checked.modules()[0].source_edge),
                "{extra}"
            );
            assert_eq!(plan.async_activations.len(), 2, "{extra}");
            assert_eq!(
                plan.async_activations
                    .values()
                    .filter(|activation| activation.origin == crate::chunks::AsyncOrigin::Import)
                    .count(),
                1,
                "{extra}"
            );
        }
    }
}

#[test]
fn lazy_certification_requires_structurally_initializer_free_options() {
    for (options, expected) in [
        ("", true),
        (", {}", true),
        (", {name: record(captured), captured}", true),
        (
            ", {'name': record(captured), \"other\": () => captured}",
            true,
        ),
        (", {initializer: initialize}", false),
        (", {initializer: undefined}", false),
        (", {'initializer': initialize}", false),
        (", {init\\u0069alizer: initialize}", false),
        (", {'\\x69nitializer': initialize}", false),
        (", {get initializer() { return initialize; }}", false),
        (", {get name() { return captured; }}", false),
        (", {name() { return captured; }}", false),
        (", {...other}", false),
        (", {[key()]: captured}", false),
        (", {__proto__: other}", false),
        (", unknownOptions", false),
        (", options()", false),
        (", undefined", false),
        (", null", false),
        (", {}, extra()", false),
        (", ...other", false),
        (", {name: captured / 2}", false),
        (", {name: /x/}", false),
    ] {
        let input = input(&format!(
            "const captured = 1; function record(x) {{ return x; }}
            const mod = new M(() => import('./target.mjs'){options});
            export const a = new R(mod, ns => ns.first);"
        ));
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            let checked = certify(&graph, &EffectPolicy::default(), &[runtime()]).unwrap();
            assert_eq!(!checked.modules().is_empty(), expected, "{options}");
        }
    }
}

#[test]
fn lazy_certification_falls_back_for_unsupported_typescript_options_on_owned_and_wire() {
    for value in ["makeLabel<First, Second>()", "<[First, Second]>captured"] {
        let mut input = input(&format!(
            "const captured = 1;
             const mod = new M(() => import('./target.mjs'), {{name: {value}}});
             export const a = new R(mod, ns => ns.first);"
        ));
        let entry = input.remove(Path::new("entry.mjs")).unwrap();
        input.insert("entry.ts".into(), entry);
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            for runtimes in [vec![], vec![runtime()]] {
                let checked = certify(&graph, &EffectPolicy::default(), &runtimes).unwrap();
                assert!(checked.modules().is_empty(), "{value}");
            }
        }
    }
}

#[test]
fn lazy_certification_rejects_nonlocal_context_commonjs_and_external_star_targets() {
    for mode in ["external", "context", "commonjs", "external-star"] {
        let mut input = input(
            "const mod = new M(() => import('./target.mjs'));
                    export const a = new R(mod, ns => ns.first);",
        );
        if mode == "commonjs" {
            input.remove(Path::new("target.mjs"));
            input.insert("target.cjs".into(), "module.exports = {first: 1};".into());
            let entry = input.get_mut(Path::new("entry.mjs")).unwrap();
            *entry = entry.replace("'./target.mjs'", "'./target.cjs'");
        } else if mode == "context" {
            let entry = input.get_mut(Path::new("entry.mjs")).unwrap();
            *entry = entry.replace("'./target.mjs'", "'./parts/' + name");
        } else if mode == "external-star" {
            input.insert(
                "target.mjs".into(),
                "export * from 'external'; export {first} from './leaf.mjs';".into(),
            );
        }
        let inputs = input
            .iter()
            .map(|(path, text)| {
                let format = if mode == "commonjs" && path == Path::new("target.cjs") {
                    ModuleFormat::CommonJs
                } else {
                    ModuleFormat::Esm
                };
                let parsed = parse_module(&logger::StdioLogger::new(), text, path, format).unwrap();
                let resolutions = parsed
                    .facts()
                    .dependencies()
                    .iter()
                    .map(|d| {
                        let target = if d.specifier() == "external" {
                            DependencyTarget::External("external".into())
                        } else if d.specifier() == "./target.mjs" && mode == "external" {
                            DependencyTarget::External("target".into())
                        } else if d.kind() == ast_segmenter::owned::DependencyKind::DynamicImport
                            && mode == "context"
                        {
                            DependencyTarget::Context(BTreeMap::from([(
                                "./parts/target.mjs".into(),
                                "target.mjs".into(),
                            )]))
                        } else {
                            DependencyTarget::File(normalize(
                                &path.parent().unwrap().join(d.specifier()),
                            ))
                        };
                        (d.id(), target)
                    })
                    .collect();
                OwnedSourceFileInput::from_parsed(path.to_path_buf(), parsed, resolutions)
            })
            .collect::<Vec<_>>();
        let graph = SourceGraph::from_owned(inputs);
        assert!(
            certify(&graph, &EffectPolicy::default(), &[runtime()])
                .unwrap()
                .modules()
                .is_empty(),
            "{mode}"
        );
    }
}

#[test]
fn lazy_certification_rejects_homonyms_shadows_and_mutable_constructor_identities() {
    for (runtime_source, body) in [
        (RUNTIME.to_owned(), "class LazyModule {}; const mod = new LazyModule(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "class LazyImport {}; const mod = new M(() => import('./target.mjs')); export const a = new LazyImport(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "function shadow(M, R) { const mod = new M(() => import('./target.mjs')); return new R(mod, ns => ns.first); }"),
        (RUNTIME.replace("export class ModuleCtor", "export let ModuleCtor = class"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (format!("{RUNTIME} ModuleCtor = class {{}};"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (format!("{RUNTIME} ImportCtor = class {{}};"), "const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "let Alias = M; const mod = new Alias(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);"),
        (RUNTIME.to_owned(), "let mod = new M(() => import('./target.mjs')); mod = other; export const a = new R(mod, ns => ns.first);"),
    ] {
        let mut input = input(body);
        let invalid_registration = runtime_source != RUNTIME;
        input.insert("runtime.mjs".into(), runtime_source);
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            let checked = certify(&graph, &EffectPolicy::default(), &[runtime()]);
            if invalid_registration {
                assert!(checked.unwrap_err().to_string().contains("invalid lazy runtime constructor identity"));
            } else {
                assert!(checked.unwrap().modules().is_empty(), "{body}");
            }
        }
    }
}

#[test]
fn lazy_certification_rejects_unsafe_target_surfaces_and_wrong_registrations() {
    for target in [
        "export const then = () => {}; export {first, second} from './leaf.mjs';",
        "export * as first from './leaf.mjs'; export {second} from './leaf.mjs';",
        "export {second} from './leaf.mjs';",
    ] {
        let mut input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
        input.insert("target.mjs".into(), target.into());
        for wire in [false, true] {
            let graph = graph_with_storage(&input, false, ModuleFormat::Esm, wire);
            assert!(certify(&graph, &EffectPolicy::default(), &[runtime()])
                .unwrap()
                .modules()
                .is_empty());
        }
    }
    let input = input("const mod = new M(() => import('./target.mjs')); export const a = new R(mod, ns => ns.first);");
    let graph = graph(&input, false);
    let mut unregistered = runtime();
    unregistered.module.module = "unrelated.mjs".into();
    assert!(certify(&graph, &EffectPolicy::default(), &[unregistered])
        .unwrap_err()
        .to_string()
        .contains("invalid lazy runtime constructor identity"));
    let mut conflicting = runtime();
    std::mem::swap(&mut conflicting.import, &mut conflicting.action);
    for registrations in [
        vec![runtime(), conflicting.clone()],
        vec![conflicting, runtime()],
    ] {
        assert!(certify(&graph, &EffectPolicy::default(), &registrations)
            .unwrap_err()
            .to_string()
            .contains("conflicting lazy runtime constructor roles"));
    }
}
