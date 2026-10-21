use super::*;
use crate::test_builder::{corrupt, FixtureRecord, RecordBuilder};

fn record(source: &str, path: &str, format: ModuleFormat) -> FixtureRecord {
    let parsed = ast_segmenter::parse_module_with_declarations(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        format,
        ecco::LocalFacts::default(),
    )
    .unwrap();
    FixtureRecord::from_parsed(source, &parsed).unwrap()
}

fn envelope(record: &FixtureRecord) -> Vec<u8> {
    record.envelope(false)
}

#[test]
fn async_deferred_roundtrips_and_rejects_missing_mutated_and_stale_facts() {
    let source = "const target = 1; async function f(value = target) { target; (() => target)(); (async () => target)(); }";
    let original = record(source, "async.mjs", ModuleFormat::Esm);
    let bytes = original.encode(source).unwrap();
    let decoded = CacheRecord::decode(&bytes, source, original.config()).unwrap();
    let parsed = decoded.to_parsed(source).unwrap();
    assert_eq!(
        parsed.facts().references(),
        original.module().facts().references()
    );
    assert_eq!(
        parsed
            .facts()
            .references()
            .iter()
            .map(|r| r.async_deferred())
            .collect::<Vec<_>>(),
        [false, true, false, true]
    );
    for index in [0, 1] {
        let changed = corrupt(&original, |v| {
            let tag = &mut v["module"]["facts"]["references"][index]["async_deferred"];
            *tag = (!tag.as_bool().unwrap()).into();
        });
        assert!(changed.encode(source).is_err());
        assert!(changed.to_parsed(source).is_err());
        assert!(CacheRecord::decode(&envelope(&changed), source, original.config()).is_err());
    }
    for (field, version) in [("schema", 12), ("extractor", 15)] {
        let stale = corrupt(&original, |v| v[field] = version.into());
        assert!(stale.to_parsed(source).is_err());
        assert!(CacheRecord::decode(&envelope(&stale), source, original.config()).is_err());
    }
    let mut missing = serde_json::to_value(&original).unwrap();
    missing["module"]["facts"]["references"][0]
        .as_object_mut()
        .unwrap()
        .remove("async_deferred");
    assert!(serde_json::from_value::<FixtureRecord>(missing).is_err());
}

#[test]
fn production_uses_typed_flatbuffers_and_rejects_previous_cbor() {
    // Old CBOR digests identify the intentionally incompatible prior format.
    for (path, source, format, previous) in [
        ("entry.mjs", "import * as ns from './dep'; export let x = 1, y = ns.value; export function f(a) { return () => a + x; } x++; /*#__PURE__*/ ns.call?.(f(y), ...items);", ModuleFormat::Esm, "19ca011458a79b6a87eaa06b0c12c16172a525743c79262b9d8826ac56d17c24"),
        ("async.mjs", "export const x = await load(); (() => { consume(x); })(); class C { [key()] = value(); static x = setup(); }", ModuleFormat::Esm, "53991301778b338305cea4e88ab8ca1a735a18ba0b8bcf9ddb5ebef7f1cba117"),
        ("entry.cjs", "#!/usr/bin/env node\n'use strict'; module.exports = name => require('./parts/' + name);", ModuleFormat::CommonJs, "76bdfcba1aa442d832757eaa11fda3ea1bdb79f329c6878cb7826f7c267d7d0a"),
        ("empty.mjs", "", ModuleFormat::Esm, "778555e065c782772c87855dc9af5d32289e1052879ef206230b481f386a146c"),
    ] {
        let bytes = record(source, path, format).encode(source).unwrap();
        assert_eq!(&bytes[8..12], b"GFSC");
        assert_ne!(format!("{:x}", Sha256::digest(&bytes)), previous, "{path}");
        let original = record(source, path, format);
        let legacy = serde_cbor::to_vec(&original).unwrap();
        assert!(CacheRecord::decode(&legacy, source, original.config()).is_err());
    }
}

#[test]
fn dynamic_namespace_observations_roundtrip_and_reject_dangling_dependencies() {
    let source =
        "export async function load() { const ns = await import('./child.mjs'); return ns.value; }";
    let record = record(source, "entry.mjs", ModuleFormat::Esm);
    let encoded = record.encode(source).unwrap();
    let decoded = CacheRecord::decode(&encoded, source, record.config()).unwrap();
    assert_eq!(
        decoded.module().facts().emission(),
        record.module().facts().emission()
    );
    assert_eq!(
        decoded
            .module()
            .facts()
            .emission()
            .dynamic_namespaces()
            .get(0)
            .unwrap()
            .properties()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [("value", false)]
    );
    let damaged = corrupt(&record, |v| {
        v.add("module.facts.emission.dynamic_namespaces.0.dependency", 1)
    });
    assert!(damaged.encode(source).is_err());
    let damaged = corrupt(&record, |v| {
        v.clear("module.facts.emission.dynamic_namespaces")
    });
    assert!(damaged.encode(source).is_err());
}

#[test]
fn provenance_rejects_repaired_source_config_and_fact_pairings() {
    let source = "export const value = 1;";
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("file.mjs"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let changed = "export const value = 2;";
    assert!(CacheRecord::from_parsed(changed, &parsed).is_err());
    let original = FixtureRecord::from_parsed(source, &parsed).unwrap();

    let replaced = corrupt(&original, |r| {
        r.set("source_hash", digest(changed.as_bytes()))
    });
    assert!(replaced.to_parsed(changed).is_err());
    assert!(CacheRecord::decode(&envelope(&replaced), changed, replaced.config()).is_err());

    let replaced = corrupt(&original, |r| r.set("config.commonjs", true));
    assert!(replaced.to_parsed(source).is_err());

    let replaced = corrupt(&original, |r| {
        r.set("module.facts.emission.segments.0.connector_only", true)
    });
    assert!(replaced.to_parsed(source).is_err());

    let (_, body, segments, facts, provenance) = parsed.into_parts();
    assert!(provenance
        .verify(
            source,
            ParserConfig::from_source_type(oxc_span::SourceType::ts()),
            body,
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
            facts.view()
        )
        .is_err());
}

#[test]
fn deterministic_roundtrip_after_parser_arena_drop() {
    for (path, source, format) in [
        ("file.ts", "#!/bin/node\n'use strict'; import {x as y} from './dep'; export const café = y; export {café as 'é'}; import('./lazy');", ModuleFormat::Esm),
        ("file.cjs", "#!/bin/node\n'use strict'; module.exports = require('./dep'); function later(){ return require('./dep'); }", ModuleFormat::CommonJs),
        ("file.d.ts", "export interface A { value: string }", ModuleFormat::Esm),
        ("empty.cjs", "", ModuleFormat::CommonJs),
        ("file.js", "export * from './a'; export * as ns from './b'; export default 1;", ModuleFormat::Esm),
        ("pure.js", "/* @__PURE__ */ object?.method?.(sideEffect(), ...iterable); const value = /*#__PURE__*/ new C(arg());", ModuleFormat::Esm),
        ("async.mjs", "export const value = await Promise.resolve(1); for await (const x of [1]) { await x; } async function deferred() { await 2; }", ModuleFormat::Esm),
        ("recipes.mjs", "import * as ns from './ns'; before; export let before = ns.value; function f(){ return before; } before++; const arrow = () => before; export default function(){ return ns.value; }", ModuleFormat::Esm),
        ("namespaces.mjs", "import * as ns from './ns'; ns.method(); ns.tag`text`; ns.value = 1; ns.value++; delete ns.value; consume(ns); ns['computed']; consume(ns.value);", ModuleFormat::Esm),
        ("named-namespace.mjs", "import {nested as alias} from './barrel'; alias.value; alias.method(); consume(alias); function shadow(alias){ return alias.value; }", ModuleFormat::Esm),
        ("default-namespace.mjs", "import alias from './barrel'; alias.value; alias.method(); consume(alias);", ModuleFormat::Esm),
        ("proto.mjs", r"let \u005f_proto__, x; const obj = {\u005f_proto__, x}; ({__proto__, x = 1} = obj); function f({x = 2}) {}", ModuleFormat::Esm),
        ("default-class.mjs", "export default class Named {}; /*# sourceURL=ignored-outside-segments */", ModuleFormat::Esm),
        ("anonymous-class.mjs", "export default class { method(){} };", ModuleFormat::Esm),
        ("empty-pattern.mjs", "export const {} = {}; export let [] = [];", ModuleFormat::Esm),
        ("default.mjs", "export default (/*# sourceURL=inside */ () => 1);", ModuleFormat::Esm),
        ("nested-pure.mjs", "async function f(){ /*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(arg), ...iterable, await later); }", ModuleFormat::Esm),
        ("chain.mjs", "/*#__PURE__*/ object.method().next?.(...iterable); /*#__PURE__*/ (object?.method)(await argument);", ModuleFormat::Esm),
        ("split.mjs", "export const café = 1, {value: next = café} = {}, last = next; let a, b = last; var c = b, d = c;", ModuleFormat::Esm),
        ("context.mjs", "export const load = name => import(`./parts/${name}.mjs`);", ModuleFormat::Esm),
        ("context.cjs", "module.exports = name => require('./parts/' + name);", ModuleFormat::CommonJs),
        ("execution.mjs", "let x = 0; const stored = (p = x++) => x++; (() => x++)(); class C { [x++] = x++; static value = x++; method(p = x++) { return () => x++; } }", ModuleFormat::Esm),
    ] {
        let first = record(source, path, format);
        let bytes = first.encode(source).unwrap();
        assert_eq!(bytes, record(source, path, format).encode(source).unwrap());
        let decoded = CacheRecord::decode(&bytes, source, first.config()).unwrap();
        assert_eq!(first, decoded);
        assert_eq!(bytes, decoded.encode(source).unwrap());
        let parsed = decoded.to_parsed(source).unwrap();
        let wire_bytes = parsed.to_flatbuffer(source, OXC_REVISION).unwrap();
        assert_eq!(wire_bytes, parsed.to_flatbuffer(source, OXC_REVISION).unwrap());
        let wire = ast_segmenter::WireRecord::new(wire_bytes.into(), source, decoded.config().wire(), OXC_REVISION).unwrap();
        let borrowed = ParsedModule::from_wire(source, wire).unwrap();
        crate::validate_parsed(source, &borrowed).unwrap();
        assert!(CacheRecord::from_parsed(source, &borrowed).is_ok());
        assert!(parsed.owned_facts().is_err());
        assert!(parsed.owned_segments().is_none());
        assert_eq!(borrowed.body(), parsed.body());
        assert_eq!(borrowed.facts().bindings().iter().map(|b| (b.id(), b.name())).collect::<Vec<_>>(), parsed.facts().bindings().iter().map(|b| (b.id(), b.name())).collect::<Vec<_>>());
        assert_eq!(
            ParserConfig::from_source_type(parsed.source_type()),
            decoded.config()
        );
    }
}

#[test]
fn rejects_corrupt_execution_contexts_and_access_timing() {
    let source =
        "let x; const stored = (p = x++) => () => x; (() => x++)(); class C { field = x++; }";
    let mutations: &[fn(&mut serde_json::Value)] = &[
        |e| {
            e.pop("execution.references");
        },
        |e| {
            e.clear("execution.invocations");
        },
        |e| e.set("execution.contexts.0.parent", Some(0)),
        |e| e.set("execution.contexts.1.parent", serde_json::Value::Null),
        |e| e.set("execution.contexts.0.span.end", u32::MAX),
        |e| e.set("execution.contexts.0.immediate", Some(u32::MAX)),
        |e| e.set("execution.contexts.3.immediate", Some(0)),
        |e| e.push("segments.1.placement.eager_writes", 0),
        |e| e.clear("segments.2.placement.eager_reads"),
    ];
    let original = record(source, "emission.mjs", ModuleFormat::Esm);
    crate::validate_emission::validate(&original.module().into(), source).unwrap();
    for (index, mutate) in mutations.iter().enumerate() {
        let mut modified = original.clone();
        modified = corrupt(&modified, |value| mutate(value.at("module.facts.emission")));
        assert!(
            crate::validate_emission::validate(&modified.module().into(), source).is_err(),
            "structural execution validation accepted mutation {index}"
        );
    }
    reject_emission(source, mutations);
}

#[test]
fn wire_semantics_reject_resealed_execution_self_parent() {
    let source = "let x; export const stored = () => () => x;";
    let parsed = record(source, "execution.mjs", ModuleFormat::Esm)
        .to_parsed(source)
        .unwrap();
    let config = parsed.provenance().config().wire();
    let wire = ast_segmenter::WireRecord::new(
        parsed.to_flatbuffer(source, OXC_REVISION).unwrap().into(),
        source,
        config,
        OXC_REVISION,
    )
    .unwrap();
    let root = wire.root();
    let module = root.module().bytes();
    let module_start = module.as_ptr() as usize - wire.bytes().as_ptr() as usize;
    let contexts = wire.module().facts().emission().execution().contexts();
    let (index, context) = contexts
        .iter()
        .enumerate()
        .find(|(_, c)| c.parent().is_some())
        .unwrap();
    let parent = context
        ._tab
        .vtable()
        .get(cache_flatbuffers::wire::ExecutionContext::VT_PARENT) as usize;
    assert_ne!(parent, 0);
    let offset = module_start + context._tab.loc() + parent;
    let seal =
        root.provenance().records_hash() as *const _ as usize - wire.bytes().as_ptr() as usize;
    let mut bytes = wire.bytes().to_vec();
    bytes[offset..offset + 4].copy_from_slice(&(index as u32).to_le_bytes());
    let hash = digest(&bytes[module_start..module_start + module.len()]);
    bytes[seal..seal + 32].copy_from_slice(&hash);
    let checksum = bytes.len() - 32;
    let hash = digest(&bytes[..checksum]);
    bytes[checksum..].copy_from_slice(&hash);

    // Structural checks and both hashes pass; source-local semantics must not.
    assert!(CacheRecord::decode(&bytes, source, parsed.provenance().config()).is_err());
    let malformed =
        ast_segmenter::WireRecord::new(bytes.into(), source, config, OXC_REVISION).unwrap();
    let borrowed = ParsedModule::from_wire(source, malformed).unwrap();
    assert!(crate::validate_parsed(source, &borrowed).is_err());
    assert!(CacheRecord::from_parsed(source, &borrowed).is_err());
}

#[test]
fn production_adapters_share_the_verified_allocation_after_record_drop() {
    let source = "export const value = 1;";
    let fixture = record(source, "input.mjs", ModuleFormat::Esm);
    let bytes = fixture.encode(source).unwrap();
    let decoded = CacheRecord::decode(&bytes, source, fixture.config()).unwrap();
    let first = decoded.to_parsed(source).unwrap();
    let second = decoded.to_parsed(source).unwrap();
    let first_name = first.facts().bindings().get(0).unwrap().name();
    let second_name = second.facts().bindings().get(0).unwrap().name();
    assert!(std::ptr::eq(first_name, second_name));
    drop(bytes);
    drop(decoded);
    assert_eq!(first_name, "value");
    assert_eq!(second_name, "value");
    assert!(first.owned_facts().is_err());
    assert!(first.owned_segments().is_none());
}

#[test]
fn production_rejects_resealed_unknown_wire_discriminant() {
    let source = "export const value = 1;";
    let record = record(source, "input.mjs", ModuleFormat::Esm);
    let bytes = record.encode(source).unwrap();
    let wire =
        ast_segmenter::WireRecord::new(bytes.into(), source, record.config().wire(), OXC_REVISION)
            .unwrap();
    let module = wire.root().module().bytes();
    let module_start = module.as_ptr() as usize - wire.bytes().as_ptr() as usize;
    let table = wire.module()._tab;
    let tag = table
        .vtable()
        .get(cache_flatbuffers::wire::CachedModule::VT_BODY_TYPE) as usize;
    assert_ne!(tag, 0);
    let seal = wire.root().provenance().records_hash() as *const _ as usize
        - wire.bytes().as_ptr() as usize;
    let mut bytes = wire.bytes().to_vec();
    bytes[module_start + table.loc() + tag] = 255;
    let hash = digest(&bytes[module_start..module_start + module.len()]);
    bytes[seal..seal + 32].copy_from_slice(&hash);
    let checksum = bytes.len() - 32;
    let hash = digest(&bytes[..checksum]);
    bytes[checksum..].copy_from_slice(&hash);
    assert!(CacheRecord::decode(&bytes, source, record.config()).is_err());
}

#[test]
fn rejects_invalid_or_repaired_multispan_sources() {
    use ast_segmenter::MultiSpan;
    assert!(MultiSpan::new([]).is_err());
    assert!(MultiSpan::new([Span::new(0, 1); 5]).is_err());
    let encoded = serde_cbor::to_vec(&vec![(0_u32, 1_u32); 5]).unwrap();
    assert!(serde_cbor::from_slice::<MultiSpan>(&encoded).is_err());
    let source = "export const a = 1, b = a;";
    let original = record(source, "file.mjs", ModuleFormat::Esm);
    let changed = corrupt(&original, |r| {
        r.set(
            "module.facts.emission.segments.1.source",
            MultiSpan::from(original.module().segments()[1]),
        )
    });
    assert!(changed.to_parsed(source).is_err());
    assert!(CacheRecord::decode(&envelope(&changed), source, original.config()).is_err());
}

fn reject_emission(source: &str, mutations: &[fn(&mut serde_json::Value)]) {
    let original = record(source, "emission.mjs", ModuleFormat::Esm);
    for (i, mutate) in mutations.iter().enumerate() {
        let mut modified = original.clone();
        modified = corrupt(&modified, |value| mutate(value.at("module.facts.emission")));
        assert!(modified.encode(source).is_err(), "encode mutation {}", i);
        assert!(
            CacheRecord::decode(&envelope(&modified), source, original.config()).is_err(),
            "decode mutation {}",
            i
        );
        assert!(
            CacheRecord::decode(&modified.envelope(true), source, original.config()).is_err(),
            "resealed wire semantics accepted mutation {i}"
        );
    }
}

#[test]
fn deferred_recipe_roundtrip_and_invalid_binding_or_source_slots() {
    let source =
        "export const first = make(), second = {first}; function read(){return second;} effect();";
    let original = record(source, "deferred.mjs", ModuleFormat::Esm);
    let bytes = original.encode(source).unwrap();
    let decoded = CacheRecord::decode(&bytes, source, original.config()).unwrap();
    assert_eq!(
        decoded.to_parsed(source).unwrap().facts().emission(),
        original.module().facts().emission()
    );
    reject_emission(
        source,
        &[
            |e| e.set("segments.0.deferred.Variable.binding", u32::MAX),
            |e| e.clear("segments.0.deferred.Variable.initializer.operations"),
            |e| e.set("segments.0.deferred.Variable.initializer.span.start", 0),
            |e| e.set("segments.0.deferred", "Function"),
            |e| e.set("segments.0.deferred", "Effect"),
        ],
    );
}

#[test]
fn rejects_malformed_emission_placement_names_and_namespace_links() {
    use ast_segmenter::{emission::*, owned::BindingId};
    let source = "import * as ns from './ns'; before; export let before = ns.value; function f(){ return before; } before++; const arrow = () => before; export default function(){ return ns.value; }";
    reject_emission(
        source,
        &[
            |e| {
                e.pop("segments");
            },
            |e| e.push("nested_bindings", BindingId(999)),
            |e| e.push("free_references", 999),
            |e| e.set("segments.0.connector_only", false),
            |e| e.clear("segments.2.placement.declarations"),
            |e| e.clear("segments.2.placement.forward_references"),
            |e| {
                e.copy(
                    "segments.2.placement.hoisted",
                    "segments.2.placement.declarations",
                )
            },
            |e| e.set("segments.3.placement.instantiable", false),
            |e| e.push("segments.2.placement.eager_reads", 999),
            |e| e.clear("segments.4.placement.writes"),
            |e| e.set("segments.6.placement.default_expression", false),
            |e| e.push("segments.3.names.named", BindingId(999)),
            |e| e.set("segments.2.namespaces.0.reference", 999),
            |e| {
                e.set(
                    "segments.2.namespaces.0.access",
                    NamespaceAccess::DirectRead {
                        member: Span::new(0, u32::MAX),
                        property: "value".into(),
                    },
                )
            },
            |e| {
                e.clear("segments.2.namespaces.0.access.DirectRead.property");
            },
            |e| {
                e.set(
                    "segments.2.namespaces.0.access",
                    NamespaceAccess::Native(NamespaceReason::Receiver),
                )
            },
        ],
    );
}

#[test]
fn rejects_missing_named_import_namespace_candidates() {
    for declaration in [
        "import {nested as alias} from './barrel';",
        "import alias from './barrel';",
    ] {
        reject_emission(
            &format!(
                "{} alias.value; alias.method(); consume(alias);",
                declaration
            ),
            &[
                |e| e.clear("segments.1.namespaces"),
                |e| e.clear("segments.2.namespaces"),
                |e| e.clear("segments.3.namespaces"),
            ],
        );
    }
}

#[test]
fn rejects_malformed_emission_operations_and_composition() {
    use ast_segmenter::{
        emission::*,
        owned::{BindingId, DependencyId},
    };
    let source = "import * as ns from './ns'; export const x = ns.value; x; import('./lazy');";
    reject_emission(
        source,
        &[
            |e| {
                e.at("segments.0.operations")
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            },
            |e| {
                let operation = e.at("segments.1.operations.0").clone();
                e.push("segments.1.operations", operation);
            },
            |e| e.set("segments.1.operations.0.parent", Some(0)),
            |e| e.set("segments.1.operations.1.parent", Some(u32::MAX)),
            |e| {
                e.at("segments.1.operations")
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1)
            },
            |e| e.clear("segments.2.operations"),
            |e| {
                e.set(
                    "segments.0.operations.0.kind",
                    OperationKind::Dependency {
                        dependency: DependencyId(999),
                    },
                )
            },
            |e| {
                e.set(
                    "segments.1.operations.1.kind",
                    OperationKind::Binding {
                        binding: BindingId(999),
                        declaration: 0,
                        shorthand: Shorthand::None,
                    },
                )
            },
            |e| {
                e.set("segments.1.operations.1.kind.Binding.declaration", 999);
            },
            |e| {
                e.set(
                    "segments.2.operations.0.kind",
                    OperationKind::Reference {
                        reference: 999,
                        shorthand: Shorthand::None,
                    },
                )
            },
            |e| {
                e.set(
                    "segments.2.operations.0.kind",
                    OperationKind::Reference {
                        reference: 2,
                        shorthand: Shorthand::Proto,
                    },
                )
            },
            |e| {
                e.set(
                    "segments.1.operations.2.kind",
                    OperationKind::Namespace { usage: 999 },
                )
            },
            |e| e.clear("segments.3.operations"),
            |e| {
                e.set(
                    "segments.2.operations.0.kind",
                    OperationKind::SourceHint {
                        span: Span::new(0, 6),
                    },
                )
            },
            |e| {
                e.set(
                    "segments.1.operations.0.kind.Export.form",
                    serde_json::json!({
                        "Default": {"value": {"span": [0, 1], "operations": []}, "hoisted": true}
                    }),
                );
            },
        ],
    );
}

fn outer_lowering(emission: &mut serde_json::Value) -> &mut serde_json::Value {
    emission
        .at("segments.0.operations")
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find_map(|operation| operation["kind"].get_mut("Invocation"))
        .unwrap()
        .at("lowering")
}

#[test]
fn rejects_malformed_pure_inputs_guards_and_source_slots() {
    let source = "/*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(arg), ...iterable, await later);";
    reject_emission(
        source,
        &[
            |e| outer_lowering(e).set("callee.span.end", u32::MAX),
            |e| {
                outer_lowering(e).pop("arguments");
            },
            |e| outer_lowering(e).add("arguments.0.span.start", 1),
            |e| outer_lowering(e).clear("arguments.0.operations"),
            |e| outer_lowering(e).clear("callee.operations"),
            |e| outer_lowering(e).add("chain.base.span.start", 1),
            |e| outer_lowering(e).clear("chain.base.operations"),
            |e| outer_lowering(e).set("guard_start", Some(0)),
            |e| outer_lowering(e).set("guard_start", Some(u32::MAX)),
            |e| outer_lowering(e).set("guard_start", serde_json::Value::Null),
            |e| outer_lowering(e).set("crosses_await_guard", false),
            |e| outer_lowering(e).clear("chain.steps"),
            |e| {
                outer_lowering(e).set("chain.steps.0.Computed.property.span.end", u32::MAX);
            },
            |e| {
                outer_lowering(e).clear("chain.steps.0.Computed.property.operations");
            },
            |e| outer_lowering(e).push("callee.operations", u32::MAX),
            |e| {
                e.set(
                    "segments.0.operations.0.kind.Invocation.invocation",
                    u32::MAX,
                );
            },
        ],
    );
}

#[test]
fn rejects_nonopaque_cjs_emission_and_stale_recipe_versions() {
    use ast_segmenter::emission::*;
    let source = "module.exports = require('./x');";
    let original = record(source, "input.cjs", ModuleFormat::CommonJs);
    for mutate in [
        |r: &mut serde_json::Value| r.set("module.facts.emission.segments.0.connector_only", true),
        |r: &mut serde_json::Value| {
            r.set(
                "module.facts.emission.segments.0.names.inference_sensitive",
                true,
            )
        },
        |r: &mut serde_json::Value| {
            r.push("module.facts.emission.segments.0.placement.eager_reads", 0)
        },
        |r: &mut serde_json::Value| {
            r.set(
                "module.facts.emission.segments.0.operations.0.kind",
                OperationKind::Reference {
                    reference: 0,
                    shorthand: Shorthand::None,
                },
            )
        },
        |r: &mut serde_json::Value| r.set("schema", 5),
        |r: &mut serde_json::Value| r.set("extractor", 8),
    ] {
        let mut modified = original.clone();
        modified = corrupt(&modified, mutate);
        assert!(modified.encode(source).is_err());
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config()).is_err());
    }
}
#[test]
fn rejects_recursive_slots_and_default_child_omission() {
    let source = "/*#__PURE__*/ f();";
    let modified = corrupt(&record(source, "input.mjs", ModuleFormat::Esm), |r| {
        r.copy(
            "module.facts.invocations.0.callee",
            "module.facts.invocations.0.span",
        );
        let whole = r.at("module.facts.invocations.0.span").clone();
        let plan = outer_lowering(r.at("module.facts.emission"));
        plan.set(
            "callee",
            serde_json::json!({"span": whole, "operations": [0]}),
        );
        plan.copy("chain.base", "callee");
    });
    assert!(modified.encode(source).is_err());
    assert!(CacheRecord::decode(&envelope(&modified), source, modified.config()).is_err());

    reject_emission(
        "export default function(){ return value; }",
        &[|e| {
            e.clear("segments.0.operations.0.kind.Export.form.Default.value.operations");
        }],
    );
    reject_emission(
        "/*#__PURE__*/ object.method(arg()).next?.(...iterable);",
        &[|e| {
            let chain = outer_lowering(e).at("chain");
            let base = chain.at("base").clone();
            for step in chain.at("steps").as_array_mut().unwrap() {
                if let Some(call) = step.get_mut("Call") {
                    call.set("suffix", base.clone());
                }
            }
        }],
    );
}

#[test]
fn rejects_versions_config_source_and_corruption() {
    let source = "export const café = 1;";
    let original = record(source, "file.js", ModuleFormat::Esm);
    for mutate in [
        |r: &mut serde_json::Value| r.add("schema", 1),
        |r: &mut serde_json::Value| r.add("extractor", 1),
        |r: &mut serde_json::Value| {
            let value = format!("{}x", r.at("oxc").as_str().unwrap());
            r.set("oxc", value);
        },
        |r: &mut serde_json::Value| r.clear("magic"),
        |r: &mut serde_json::Value| r.add("source_len", 1),
        |r: &mut serde_json::Value| {
            let value = r.at("source_hash.0").as_u64().unwrap() ^ 1;
            r.set("source_hash.0", value);
        },
        |r: &mut serde_json::Value| r.set("config.jsx", true),
    ] {
        let mut modified = original.clone();
        modified = corrupt(&modified, mutate);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config()).is_err());
    }
    let bytes = original.encode(source).unwrap();
    assert!(CacheRecord::decode(&bytes, "export const café = 2;", original.config()).is_err());
    for i in 0..bytes.len() {
        assert!(CacheRecord::decode(&bytes[..i], source, original.config()).is_err());
        let mut corrupt = bytes.clone();
        corrupt[i] ^= 1;
        assert!(
            CacheRecord::decode(&corrupt, source, original.config()).is_err(),
            "byte {i}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(CacheRecord::decode(&trailing, source, original.config()).is_err());
}

#[test]
fn rejects_impossible_spans_ids_and_order() {
    let source = "export const café = 1; console.log(café);";
    let original = record(source, "file.js", ModuleFormat::Esm);
    for mutate in [
        |r: &mut serde_json::Value| r.set("module.segments.0", [2, 1]),
        |r: &mut serde_json::Value| r.set("module.segments.0.end", u32::MAX),
        |r: &mut serde_json::Value| r.set("module.facts.bindings.0.declarations.0.end", 17),
        |r: &mut serde_json::Value| r.add("module.facts.bindings.0.id", 1),
        |r: &mut serde_json::Value| {
            r.set(
                "module.facts.references.0.binding",
                Some(ast_segmenter::owned::BindingId(999)),
            )
        },
        |r: &mut serde_json::Value| r.reverse("module.segments"),
        |r: &mut serde_json::Value| r.set("module.facts.commonjs", true),
        |r: &mut serde_json::Value| r.set("module.facts.effects.0.span.end", u32::MAX),
        |r: &mut serde_json::Value| r.reverse("module.facts.effects"),
        |r: &mut serde_json::Value| r.set("module.facts.effects.0.deferred", [0, 1]),
        |r: &mut serde_json::Value| r.set("module.facts.invocations.0.callee", [0, 1]),
        |r: &mut serde_json::Value| {
            r.set("module.facts.invocations.0.arguments.0.span.end", u32::MAX)
        },
        |r: &mut serde_json::Value| r.set("module.facts.invocations.0.pure", [0, 6]),
        |r: &mut serde_json::Value| r.set("module.facts.invocations.0.deferred", [0, 1]),
    ] {
        let mut modified = original.clone();
        modified = corrupt(&modified, mutate);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config()).is_err());
        assert!(CacheRecord::decode(&modified.envelope(true), source, original.config()).is_err());
    }
}

#[test]
fn flatbuffer_decoder_rejects_legacy_allocation_and_depth_bombs() {
    for bytes in [
        vec![0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x9f, 0xff],
        vec![0x81; 40],
        vec![0; MAX_RECORD_BYTES + 1],
        [vec![0x9a, 0, 32, 0, 0], vec![0; 2_097_152]].concat(),
    ] {
        let config = ParserConfig::for_path(Path::new("empty.mjs"), ModuleFormat::Esm).unwrap();
        assert!(CacheRecord::decode(&bytes, "", config).is_err());
    }
}

#[test]
fn rejects_invalid_async_syntax_sites() {
    let source = "await 1; await 2;";
    let original = record(source, "async.mjs", ModuleFormat::Esm);
    assert_eq!(original.module().facts().top_level_await().len(), 2);
    for mutate in [
        |r: &mut serde_json::Value| r.reverse("module.facts.top_level_await"),
        |r: &mut serde_json::Value| r.set("module.facts.top_level_await.0.end", u32::MAX),
        |r: &mut serde_json::Value| r.set("module.facts.top_level_await.0", [8, 8]),
        |r: &mut serde_json::Value| r.set("schema", 4),
        |r: &mut serde_json::Value| r.set("extractor", 5),
    ] {
        let mut modified = original.clone();
        modified = corrupt(&modified, mutate);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config()).is_err());
        assert!(CacheRecord::decode(&modified.envelope(true), source, original.config()).is_err());
    }
}
