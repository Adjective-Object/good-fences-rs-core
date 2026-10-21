use super::*;
use ecco::Hazard;
use serde_json::{Value as Json, json};

fn modules(files: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
    files
        .iter()
        .map(|(path, source)| (PathBuf::from(path), (*source).into()))
        .collect()
}

fn execute(source: &str) -> HarnessRunResult {
    run(&modules(&[("entry.js", source)]), "entry.js").unwrap()
}

fn effect(name: &str, arguments: Json) -> Effect {
    Effect {
        name: name.into(),
        arguments: arguments.as_array().unwrap().clone(),
    }
}

#[test]
fn executes_every_ecco_hazard_with_exact_arguments() {
    let cases = [
        (
            Hazard::PropertyRead,
            "property('p', 9).value",
            vec![effect("property.get", json!(["p"]))],
        ),
        (
            Hazard::ToPrimitive,
            "+coercible('c', 3)",
            vec![effect("coerce", json!(["c", "number"]))],
        ),
        (
            Hazard::In,
            "'key' in membership('m')",
            vec![effect("has", json!(["m", "key"]))],
        ),
        (
            Hazard::Instanceof,
            "17 instanceof instance('i')",
            vec![effect("instanceof", json!(["i", 17]))],
        ),
        (
            Hazard::Call,
            "callable('f', 1)(2, 3)",
            vec![effect("call", json!(["f", null, 2, 3]))],
        ),
        (
            Hazard::Construct,
            "new (constructible('c'))(2, 3)",
            vec![effect("construct", json!(["c", 2, 3]))],
        ),
        (
            Hazard::Iterate,
            "[...iterable('it', [4])]",
            vec![
                effect("iterator", json!(["it"])),
                effect("next", json!(["it", 0])),
                effect("next", json!(["it", 1])),
            ],
        ),
        (
            Hazard::PrototypeRead,
            "prototype('p') instanceof Object",
            vec![effect("prototype", json!(["p"]))],
        ),
    ];
    for hazard in Hazard::ALL {
        let (_, source, expected) = cases
            .iter()
            .find(|(covered, _, _)| *covered == hazard)
            .unwrap_or_else(|| panic!("missing runtime trigger for {hazard:?}"));
        let result = execute(&format!(
            "import {{ property, coercible, membership, instance, callable,
                       constructible, iterable, prototype }} from 'harness'; {source};"
        ));
        assert_eq!(&result.effects, expected, "{hazard:?}");
    }
}

#[test]
fn pure_controls_and_fixture_creation_do_not_run_operations() {
    let result = execute(
        r#"
        import * as h from 'harness';
        h.property('p', 1); h.coercible('c', 1); h.membership('m');
        h.instance('i'); h.callable('f', 1); h.constructible('c');
        h.iterable('it', [1]); h.prototype('p');
        const object = {};
        h.effect('controls', h.pure(object) === object, ({value: 1}).value,
          +3, 'key' in {}, 1 instanceof Object, (() => 2)(),
          new Object() !== new Object(), [...[4]][0],
          Object.getPrototypeOf({}) === Object.prototype);
        "#,
    );
    assert_eq!(
        result.effects,
        vec![effect(
            "controls",
            json!([true, 1, 3, false, false, 2, true, 4, true])
        )]
    );
}

#[test]
fn arguments_are_owned_snapshots_not_references() {
    let result = execute(
        r#"
        import { effect } from 'harness';
        const value = { nested: [1, null, true, "text"], label: "before" };
        effect('before', value, value);
        value.nested[0] = 2;
        value.label = "after";
        effect('after', value);
        "#,
    );
    assert_eq!(
        result.effects,
        vec![
            effect(
                "before",
                json!([
                    {"nested": [1, null, true, "text"], "label": "before"},
                    {"nested": [1, null, true, "text"], "label": "before"}
                ])
            ),
            effect(
                "after",
                json!([{"nested": [2, null, true, "text"], "label": "after"}])
            ),
        ]
    );
}

#[test]
fn serialization_failure_is_explicit_even_if_caught() {
    for argument in [
        "undefined",
        "() => {}",
        "Symbol('s')",
        "1n",
        "NaN",
        "Infinity",
        "({ nested: undefined })",
        "({ toJSON() { return undefined; } })",
        "[undefined]",
        "Array(1)",
        "'\\ud800'",
        "(() => { const x = {}; x.self = x; return x; })()",
    ] {
        let source = format!(
            "import {{ effect }} from 'harness'; \
             try {{ effect('unsupported', {argument}); }} catch {{}}"
        );
        assert!(
            run(&modules(&[("entry.js", &source)]), "entry.js").is_err(),
            "{argument}"
        );
    }
}

#[test]
fn malformed_names_fail_the_run_even_if_caught_with_supported_or_unsupported_arguments() {
    for name in [r"'\ud800'", r"'\udfff'"] {
        for argument in ["1", "undefined"] {
            let source = format!(
                "import {{ effect }} from 'harness'; \
                 try {{ effect({name}, {argument}); }} catch {{}} effect('after');"
            );
            let error = run(&modules(&[("entry.js", &source)]), "entry.js").unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("effect name must be a valid Unicode string"),
                "{name}, {argument}: {error:#}"
            );
        }
    }
    assert_eq!(
        execute(r"import { effect } from 'harness'; effect('\ud83d\ude00', 1);").effects,
        vec![effect("😀", json!([1]))]
    );
}

#[test]
fn native_payload_conversion_failures_are_sticky() {
    for payload in [r"'\ud800'", "123"] {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        let exhausted = Rc::new(RefCell::new(None));
        let state = Rc::new(RefCell::new(Recorder {
            effects: Vec::new(),
            values: 0,
            bytes: 0,
            limits: Limits::default(),
            exhausted: exhausted.clone(),
        }));
        context.with(|ctx| {
            ctx.globals()
                .set(
                    "record",
                    recorder_function(ctx.clone(), state.clone()).unwrap(),
                )
                .unwrap();
            ctx.eval::<(), _>(format!("try {{ record('name', {payload}); }} catch {{}}"))
                .unwrap();
        });
        assert_eq!(
            exhausted.borrow().as_deref(),
            Some("serialized effect arguments must be a valid Unicode string"),
            "{payload}"
        );
        assert!(state.borrow().effects.is_empty());
    }
}

#[test]
fn serialization_uses_captured_json_and_has_documented_json_hooks() {
    let result = execute(
        r#"
        import { effect } from 'harness';
        JSON.stringify = () => { throw 'replaced'; };
        effect('outer', {
          get value() { effect('getter'); return 1; },
          converted: { toJSON() { effect('toJSON'); return "converted"; } }
        });
        "#,
    );
    assert_eq!(
        result.effects,
        vec![
            effect("getter", json!([])),
            effect("toJSON", json!([])),
            effect("outer", json!([{"value": 1, "converted": "converted"}])),
        ]
    );
}

#[test]
fn callbacks_receivers_rest_live_aliases_captures_and_return_patterns() {
    let result = execute(
        r#"
        import { effect, property, invoke, cell, callable, constructible,
                 globalRead, fail } from 'harness';
        const receiver = { tag: 'receiver' }, shared = cell(1), alias = shared;
        const read = () => shared.value;
        function callback(...args) {
          effect('callback', this, ...args);
          alias.value = args[1];
          return read();
        }
        effect('result', invoke(callback, receiver, 2, 3), read());
        const accessor = property('p', 4);
        accessor.value = 5;
        effect('read', accessor.value);
        const identity = callable('id', receiver);
        effect('identity', invoke(identity, null) === receiver);
        const Fresh = constructible('fresh');
        const one = new Fresh(6), two = new Fresh(7);
        effect('fresh', one, two, one !== two);
        effect('void', typeof globalRead('ambient', 8));
        effect('global-result', ambient, typeof ambient);
        try { fail('f', receiver); }
        catch (error) { effect('caught', error === receiver); }
        "#,
    );
    assert_eq!(
        result.effects,
        vec![
            effect("callback", json!([{"tag": "receiver"}, 2, 3])),
            effect("result", json!([3, 3])),
            effect("property.set", json!(["p", 5])),
            effect("property.get", json!(["p"])),
            effect("read", json!([5])),
            effect("call", json!(["id", null])),
            effect("identity", json!([true])),
            effect("construct", json!(["fresh", 6])),
            effect("construct", json!(["fresh", 7])),
            effect("fresh", json!([{}, {}, true])),
            effect("void", json!(["undefined"])),
            effect("global", json!(["ambient"])),
            effect("global", json!(["ambient"])),
            effect("global-result", json!([8, "number"])),
            effect("fail", json!(["f", {"tag": "receiver"}])),
            effect("caught", json!([true])),
        ]
    );
}

#[test]
fn iterator_closure_runs_before_following_effects() {
    let result = execute(
        r#"
        import { effect, iterable } from 'harness';
        for (const value of iterable('i', [10, 20])) {
          effect('body', value);
          break;
        }
        effect('after');
        "#,
    );
    assert_eq!(
        result.effects,
        vec![
            effect("iterator", json!(["i"])),
            effect("next", json!(["i", 0])),
            effect("body", json!([10])),
            effect("return", json!(["i"])),
            effect("after", json!([])),
        ]
    );
}

#[test]
fn static_dynamic_imports_tla_and_live_module_identity() {
    let result = run(
        &modules(&[
            (
                "src/entry.js",
                r#"
                import { effect } from 'harness';
                import { value, change } from './dependency.js';
                effect('entry', value);
                const first = await import('././dependency.js');
                change();
                const second = await import('../src/dependency.js');
                effect('dynamic', first === second, first.value, value);
                await Promise.resolve();
                effect('done');
                "#,
            ),
            (
                "src/dependency.js",
                r#"
                import { effect } from 'harness';
                export let value = 1;
                export function change() { value = 2; }
                effect('start');
                await Promise.resolve();
                effect('dependency', value);
                "#,
            ),
        ]),
        "src/entry.js",
    )
    .unwrap();
    assert_eq!(
        result.effects,
        vec![
            effect("start", json!([])),
            effect("dependency", json!([1])),
            effect("entry", json!([1])),
            effect("dynamic", json!([true, 2, 2])),
            effect("done", json!([])),
        ]
    );
}

#[test]
fn native_module_cycles_preserve_hoisted_functions_and_effect_order() {
    let result = run(
        &modules(&[
            (
                "entry.js",
                r#"
                import { effect } from 'harness';
                import { ping } from './a.js';
                effect('entry', ping());
            "#,
            ),
            (
                "a.js",
                r#"
                import { effect } from 'harness';
                import { value } from './b.js';
                export function ping() { return value; }
                effect('a');
            "#,
            ),
            (
                "b.js",
                r#"
                import { effect } from 'harness';
                import { ping } from './a.js';
                export const value = 2;
                effect('b', typeof ping);
            "#,
            ),
        ]),
        "entry.js",
    )
    .unwrap();
    assert_eq!(
        result.effects,
        vec![
            effect("b", json!(["function"])),
            effect("a", json!([])),
            effect("entry", json!([2])),
        ]
    );
}

#[test]
fn cached_module_failure_including_throw_undefined() {
    let result = run(
        &modules(&[
            (
                "entry.js",
                r#"
                import { effect } from 'harness';
                for (let i = 0; i < 2; i++) {
                  try { await import('./bad.js'); }
                  catch (reason) { effect('caught', typeof reason); }
                }
                "#,
            ),
            (
                "bad.js",
                "import { effect } from 'harness'; effect('init'); throw undefined;",
            ),
        ]),
        "entry.js",
    )
    .unwrap();
    assert_eq!(
        result.effects,
        vec![
            effect("init", json!([])),
            effect("caught", json!(["undefined"])),
            effect("caught", json!(["undefined"])),
        ]
    );
}

#[test]
fn runs_are_fresh_and_have_no_host_capabilities() {
    let source = r#"
        import { effect, globalRead } from 'harness';
        effect('fresh', typeof ambient);
        globalRead('ambient', 1);
        effect('host', typeof process, typeof require, typeof std, typeof os,
          typeof fetch, typeof console, typeof __quickjs_harness_record);
    "#;
    let expected = vec![
        effect("fresh", json!(["undefined"])),
        effect("host", json!(vec!["undefined"; 7])),
    ];
    assert_eq!(execute(source).effects, expected);
    assert_eq!(execute(source).effects, expected);
}

#[test]
fn rejects_external_missing_absolute_and_escaping_imports() {
    for name in [
        "node:fs",
        "std",
        "os",
        "fs",
        "https://example.com/module.js",
        "/etc/passwd",
        "./missing.js",
        "../other-example/module.js",
        "./a\\\\b.js",
    ] {
        for source in [
            format!("import '{name}';"),
            format!("await import('{name}');"),
        ] {
            assert!(
                run(&modules(&[("entry.js", &source)]), "entry.js").is_err(),
                "{source}"
            );
        }
    }
    assert!(run(&modules(&[("harness", "")]), "harness").is_err());
    assert!(run(&modules(&[("../outside.js", "")]), "../outside.js").is_err());
    assert!(
        run(
            &modules(&[("entry.js", ""), ("./entry.js", "")]),
            "entry.js"
        )
        .is_err()
    );
    assert!(run(&modules(&[("entry.js", "")]), "missing.js").is_err());
}

#[test]
fn exceptions_and_unhandled_rejections_are_not_empty_success() {
    for source in [
        "throw undefined",
        "await Promise.reject(undefined)",
        "Promise.reject(undefined)",
        "Promise.resolve().then(() => { throw undefined; })",
    ] {
        let error = run(&modules(&[("entry.js", source)]), "entry.js").unwrap_err();
        assert!(error.to_string().contains("undefined"), "{error:#}");
    }
    let syntax = run(&modules(&[("entry.js", "export const = ;")]), "entry.js").unwrap_err();
    assert!(
        !syntax.to_string().contains("<thrown"),
        "missing syntax diagnostic: {syntax:#}"
    );
    let error = run(
        &modules(&[(
            "entry.js",
            r#"
            const error = new Error('original');
            Object.defineProperty(error, 'message', { get() { while (true) {} } });
            error.toString = () => { while (true) {} };
            throw error;
        "#,
        )]),
        "entry.js",
    )
    .unwrap_err();
    assert!(error.to_string().contains("<thrown"), "{error:#}");
    assert_eq!(
        execute(
            "import {effect} from 'harness'; \
         Promise.reject(1).catch(value => effect('handled', value));"
        )
        .effects,
        vec![effect("handled", json!([1]))]
    );
}

fn limited_error(source: &str, limits: Limits) -> String {
    run_with_limits(&modules(&[("entry.js", source)]), "entry.js", limits)
        .unwrap_err()
        .to_string()
}

#[test]
fn bounded_execution_and_unsettled_entries_fail_explicitly() {
    assert!(
        limited_error(
            "while (true) {}",
            Limits {
                interrupt_ticks: 20,
                ..Limits::default()
            }
        )
        .contains("instruction/interrupt budget")
    );
    assert!(
        limited_error(
            "function next() { Promise.resolve().then(next); } next();",
            Limits {
                jobs: 5,
                ..Limits::default()
            }
        )
        .contains("pending-job budget")
    );
    assert!(
        limited_error("await new Promise(() => {});", Limits::default())
            .contains("entry evaluation is unsettled")
    );
    assert!(
        limited_error("new ArrayBuffer(100_000_000);", Limits::default()).contains("out of memory")
    );
    assert!(
        limited_error(
            "function recurse() { recurse(); } recurse();",
            Limits::default()
        )
        .contains("stack")
    );
}

#[test]
fn output_and_input_budgets_fail_even_when_guest_catches_recorder_errors() {
    assert!(
        limited_error(
            "import {effect} from 'harness'; try { effect('a'); effect('b'); } catch {}",
            Limits {
                effects: 1,
                ..Limits::default()
            }
        )
        .contains("effect budget")
    );
    assert!(
        limited_error(
            "import {effect} from 'harness'; effect('a', {nested: [1, 2]});",
            Limits {
                values: 1,
                ..Limits::default()
            }
        )
        .contains("recorded value budget")
    );
    assert!(
        limited_error(
            "import {effect} from 'harness'; effect('a', 'large');",
            Limits {
                recorded_bytes: 5,
                ..Limits::default()
            }
        )
        .contains("recorded byte budget")
    );
    assert!(
        limited_error(
            "",
            Limits {
                source_bytes: 1,
                ..Limits::default()
            }
        )
        .contains("source byte budget")
    );
    assert!(
        limited_error(
            "",
            Limits {
                modules: 0,
                ..Limits::default()
            }
        )
        .contains("module count budget")
    );
}

#[test]
fn actual_native_bundler_output_preserves_source_effects() {
    use ast_segmenter::{ModuleFormat, parse_module};
    use bundler::{
        chunks::{RootId, RootSeed, partition},
        link::{Backend, esm},
        policy::EffectPolicy,
    };
    use source_graph::{DependencyTarget, OwnedSourceFileInput, SourceGraph};

    let sources = modules(&[
        (
            "entry.js",
            r#"
            import { effect } from 'harness';
            import { value, update } from './dependency.js';
            const unused = 123;
            effect('entry', value);
            update();
            effect('updated', value);
        "#,
        ),
        (
            "dependency.js",
            r#"
            import { effect } from 'harness';
            export let value = 1;
            export function update() { value = 2; }
            effect('dependency');
        "#,
        ),
    ]);
    let inputs = ["entry.js", "dependency.js"].map(|path| {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            &sources[Path::new(path)],
            Path::new(path),
            ModuleFormat::Esm,
        )
        .unwrap();
        let resolutions = parsed
            .facts
            .dependencies
            .iter()
            .map(|dependency| {
                let target = if dependency.specifier == "harness" {
                    DependencyTarget::External("harness".into())
                } else {
                    DependencyTarget::File("dependency.js".into())
                };
                (dependency.id, target)
            })
            .collect();
        OwnedSourceFileInput::from_parsed(path.into(), parsed, resolutions)
    });
    let graph = SourceGraph::from_owned(inputs);
    let policy = EffectPolicy::default();
    let plan = partition(
        &graph,
        &[RootSeed {
            id: RootId::Entry("main".into()),
            evaluation_roots: vec![0],
            binding_roots: vec![],
        }],
        &policy,
    )
    .unwrap();
    let artifacts = esm(&graph, &plan, &policy, &sources).unwrap();
    assert_eq!(artifacts.backend, Backend::Native);
    let original = run(&sources, "entry.js").unwrap();
    assert_eq!(
        original.effects,
        vec![
            effect("dependency", json!([])),
            effect("entry", json!([1])),
            effect("updated", json!([2])),
        ]
    );
    let emitted = run(&artifacts.files, &artifacts.entries["main"]).unwrap();
    assert_eq!(original, emitted);
}
