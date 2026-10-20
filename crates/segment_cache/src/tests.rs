use super::*;

fn record(source: &str, path: &str, format: ModuleFormat) -> CacheRecord {
    let parsed =
        ast_segmenter::parse_module(&logger::StdioLogger::new(), source, Path::new(path), format)
            .unwrap();
    CacheRecord::from_parsed(source, &parsed).unwrap()
}

fn envelope(record: &CacheRecord) -> Vec<u8> {
    let payload = serde_cbor::to_vec(record).unwrap();
    serde_cbor::to_vec(&Envelope {
        checksum: digest(&payload),
        payload,
    })
    .unwrap()
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
        ("proto.mjs", r"let \u005f_proto__, x; const obj = {\u005f_proto__, x}; ({__proto__, x = 1} = obj); function f({x = 2}) {}", ModuleFormat::Esm),
        ("default-class.mjs", "export default class Named {}; /*# sourceURL=ignored-outside-segments */", ModuleFormat::Esm),
        ("anonymous-class.mjs", "export default class { method(){} };", ModuleFormat::Esm),
        ("empty-pattern.mjs", "export const {} = {}; export let [] = [];", ModuleFormat::Esm),
        ("default.mjs", "export default (/*# sourceURL=inside */ () => 1);", ModuleFormat::Esm),
        ("nested-pure.mjs", "async function f(){ /*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(arg), ...iterable, await later); }", ModuleFormat::Esm),
        ("chain.mjs", "/*#__PURE__*/ object.method().next?.(...iterable); /*#__PURE__*/ (object?.method)(await argument);", ModuleFormat::Esm),
    ] {
        let first = record(source, path, format);
        let bytes = first.encode(source).unwrap();
        assert_eq!(bytes, record(source, path, format).encode(source).unwrap());
        let decoded = CacheRecord::decode(&bytes, source, first.config).unwrap();
        assert_eq!(first, decoded);
        assert_eq!(bytes, decoded.encode(source).unwrap());
    }
}

fn reject_emission(source: &str, mutations: &[fn(&mut ast_segmenter::emission::EmissionFacts)]) {
    let original = record(source, "emission.mjs", ModuleFormat::Esm);
    for (i, mutate) in mutations.iter().enumerate() {
        let mut modified = original.clone();
        mutate(&mut modified.module.facts.emission);
        assert!(modified.encode(source).is_err(), "encode mutation {}", i);
        assert!(
            CacheRecord::decode(&envelope(&modified), source, original.config).is_err(),
            "decode mutation {}",
            i
        );
    }
}

#[test]
fn rejects_malformed_emission_placement_names_and_namespace_links() {
    use ast_segmenter::{emission::*, owned::BindingId};
    let source = "import * as ns from './ns'; before; export let before = ns.value; function f(){ return before; } before++; const arrow = () => before; export default function(){ return ns.value; }";
    reject_emission(
        source,
        &[
            |e| {
                e.segments.pop();
            },
            |e| e.nested_bindings.push(BindingId(999)),
            |e| e.free_references.push(999),
            |e| e.segments[0].connector_only = false,
            |e| e.segments[2].placement.declarations.clear(),
            |e| e.segments[2].placement.forward_references.clear(),
            |e| e.segments[2].placement.hoisted = e.segments[2].placement.declarations.clone(),
            |e| e.segments[3].placement.instantiable = false,
            |e| e.segments[2].placement.eager_reads.push(999),
            |e| e.segments[4].placement.writes.clear(),
            |e| e.segments[6].placement.default_expression = false,
            |e| e.segments[3].names.named.push(BindingId(999)),
            |e| e.segments[2].namespaces[0].reference = 999,
            |e| {
                e.segments[2].namespaces[0].access = NamespaceAccess::DirectRead {
                    member: Span::new(0, u32::MAX),
                    property: "value".into(),
                }
            },
            |e| {
                if let NamespaceAccess::DirectRead { property, .. } =
                    &mut e.segments[2].namespaces[0].access
                {
                    property.clear();
                }
            },
            |e| {
                e.segments[2].namespaces[0].access =
                    NamespaceAccess::Native(NamespaceReason::Receiver)
            },
        ],
    );
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
                e.segments[0].operations.remove(0);
            },
            |e| {
                let operation = e.segments[1].operations[0].clone();
                e.segments[1].operations.push(operation);
            },
            |e| e.segments[1].operations[0].parent = Some(0),
            |e| e.segments[1].operations[1].parent = Some(u32::MAX),
            |e| e.segments[1].operations.swap(0, 1),
            |e| e.segments[2].operations.clear(),
            |e| {
                e.segments[0].operations[0].kind = OperationKind::Dependency {
                    dependency: DependencyId(999),
                }
            },
            |e| {
                e.segments[1].operations[1].kind = OperationKind::Binding {
                    binding: BindingId(999),
                    declaration: 0,
                    shorthand: Shorthand::None,
                }
            },
            |e| {
                if let OperationKind::Binding { declaration, .. } =
                    &mut e.segments[1].operations[1].kind
                {
                    *declaration = 999;
                }
            },
            |e| {
                e.segments[2].operations[0].kind = OperationKind::Reference {
                    reference: 999,
                    shorthand: Shorthand::None,
                }
            },
            |e| {
                e.segments[2].operations[0].kind = OperationKind::Reference {
                    reference: 2,
                    shorthand: Shorthand::Proto,
                }
            },
            |e| e.segments[1].operations[2].kind = OperationKind::Namespace { usage: 999 },
            |e| e.segments[3].operations.clear(),
            |e| {
                e.segments[2].operations[0].kind = OperationKind::SourceHint {
                    span: Span::new(0, 6),
                }
            },
            |e| {
                if let OperationKind::Export { form, .. } = &mut e.segments[1].operations[0].kind {
                    *form = ExportForm::Default {
                        value: SourceSlot {
                            span: Span::new(0, 1),
                            operations: vec![],
                        },
                        hoisted: true,
                    };
                }
            },
        ],
    );
}

fn outer_lowering(
    emission: &mut ast_segmenter::emission::EmissionFacts,
) -> &mut ast_segmenter::emission::PureLowering {
    emission.segments[0]
        .operations
        .iter_mut()
        .find_map(|operation| {
            if let ast_segmenter::emission::OperationKind::Invocation { lowering, .. } =
                &mut operation.kind
            {
                Some(lowering)
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn rejects_malformed_pure_inputs_guards_and_source_slots() {
    use ast_segmenter::emission::*;
    let source = "/*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(arg), ...iterable, await later);";
    reject_emission(
        source,
        &[
            |e| outer_lowering(e).callee.span.end = u32::MAX,
            |e| {
                outer_lowering(e).arguments.pop();
            },
            |e| outer_lowering(e).arguments[0].span.start += 1,
            |e| outer_lowering(e).arguments[0].operations.clear(),
            |e| outer_lowering(e).callee.operations.clear(),
            |e| outer_lowering(e).chain.as_mut().unwrap().base.span.start += 1,
            |e| {
                outer_lowering(e)
                    .chain
                    .as_mut()
                    .unwrap()
                    .base
                    .operations
                    .clear()
            },
            |e| outer_lowering(e).guard_start = Some(0),
            |e| outer_lowering(e).guard_start = Some(u32::MAX),
            |e| outer_lowering(e).guard_start = None,
            |e| outer_lowering(e).crosses_await_guard = false,
            |e| outer_lowering(e).chain.as_mut().unwrap().steps.clear(),
            |e| {
                if let ChainStep::Computed { property, .. } =
                    &mut outer_lowering(e).chain.as_mut().unwrap().steps[0]
                {
                    property.span.end = u32::MAX;
                }
            },
            |e| {
                if let ChainStep::Computed { property, .. } =
                    &mut outer_lowering(e).chain.as_mut().unwrap().steps[0]
                {
                    property.operations.clear();
                }
            },
            |e| outer_lowering(e).callee.operations.push(u32::MAX),
            |e| {
                if let OperationKind::Invocation { invocation, .. } =
                    &mut e.segments[0].operations[0].kind
                {
                    *invocation = u32::MAX;
                }
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
        |r: &mut CacheRecord| r.module.facts.emission.segments[0].connector_only = true,
        |r: &mut CacheRecord| {
            r.module.facts.emission.segments[0]
                .names
                .inference_sensitive = true
        },
        |r: &mut CacheRecord| {
            r.module.facts.emission.segments[0]
                .placement
                .eager_reads
                .push(0)
        },
        |r: &mut CacheRecord| {
            r.module.facts.emission.segments[0].operations[0].kind = OperationKind::Reference {
                reference: 0,
                shorthand: Shorthand::None,
            }
        },
        |r: &mut CacheRecord| r.schema = 5,
        |r: &mut CacheRecord| r.extractor = 7,
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(modified.encode(source).is_err());
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
}
#[test]
fn rejects_recursive_slots_and_default_child_omission() {
    use ast_segmenter::emission::*;
    let source = "/*#__PURE__*/ f();";
    let mut modified = record(source, "input.mjs", ModuleFormat::Esm);
    let call = &mut modified.module.facts.invocations[0];
    call.callee = call.span;
    let whole = call.span;
    let plan = outer_lowering(&mut modified.module.facts.emission);
    plan.callee = SourceSlot {
        span: whole,
        operations: vec![0],
    };
    plan.chain.as_mut().unwrap().base = plan.callee.clone();
    assert!(modified.encode(source).is_err());
    assert!(CacheRecord::decode(&envelope(&modified), source, modified.config).is_err());

    reject_emission(
        "export default function(){ return value; }",
        &[|e| {
            if let OperationKind::Export {
                form: ExportForm::Default { value, .. },
                ..
            } = &mut e.segments[0].operations[0].kind
            {
                value.operations.clear();
            }
        }],
    );
    reject_emission(
        "/*#__PURE__*/ object.method(arg()).next?.(...iterable);",
        &[|e| {
            let chain = outer_lowering(e).chain.as_mut().unwrap();
            let base = chain.base.clone();
            for step in &mut chain.steps {
                if let ChainStep::Call { suffix } = step {
                    *suffix = base.clone();
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
        |r: &mut CacheRecord| r.schema += 1,
        |r: &mut CacheRecord| r.extractor += 1,
        |r: &mut CacheRecord| r.oxc.push('x'),
        |r: &mut CacheRecord| r.magic.clear(),
        |r: &mut CacheRecord| r.source_len += 1,
        |r: &mut CacheRecord| r.source_hash[0] ^= 1,
        |r: &mut CacheRecord| r.config.jsx = true,
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
    let bytes = original.encode(source).unwrap();
    assert!(CacheRecord::decode(&bytes, "export const café = 2;", original.config).is_err());
    for i in 0..bytes.len() {
        assert!(CacheRecord::decode(&bytes[..i], source, original.config).is_err());
        let mut corrupt = bytes.clone();
        corrupt[i] ^= 1;
        assert!(
            CacheRecord::decode(&corrupt, source, original.config).is_err(),
            "byte {i}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(CacheRecord::decode(&trailing, source, original.config).is_err());
}

#[test]
fn rejects_impossible_spans_ids_and_order() {
    let source = "export const café = 1; console.log(café);";
    let original = record(source, "file.js", ModuleFormat::Esm);
    for mutate in [
        |r: &mut CacheRecord| r.module.segments[0] = Span::new(2, 1),
        |r: &mut CacheRecord| r.module.segments[0].end = u32::MAX,
        |r: &mut CacheRecord| r.module.facts.bindings[0].declarations[0].end = 17,
        |r: &mut CacheRecord| r.module.facts.bindings[0].id.0 += 1,
        |r: &mut CacheRecord| {
            r.module.facts.references[0].binding = Some(ast_segmenter::owned::BindingId(999))
        },
        |r: &mut CacheRecord| r.module.segments.reverse(),
        |r: &mut CacheRecord| r.module.facts.commonjs = true,
        |r: &mut CacheRecord| r.module.facts.effects[0].span.end = u32::MAX,
        |r: &mut CacheRecord| r.module.facts.effects.reverse(),
        |r: &mut CacheRecord| r.module.facts.effects[0].deferred = Some(Span::new(0, 1)),
        |r: &mut CacheRecord| r.module.facts.invocations[0].callee = Span::new(0, 1),
        |r: &mut CacheRecord| r.module.facts.invocations[0].arguments[0].span.end = u32::MAX,
        |r: &mut CacheRecord| r.module.facts.invocations[0].pure = Some(Span::new(0, 6)),
        |r: &mut CacheRecord| r.module.facts.invocations[0].deferred = Some(Span::new(0, 1)),
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
}

#[test]
fn bounded_decoder_rejects_allocation_and_depth_bombs() {
    for bytes in [
        vec![0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x9f, 0xff],
        vec![0x81; 40],
        vec![0; MAX_RECORD_BYTES + 1],
        [vec![0x9a, 0, 4, 0, 0], vec![0; 262_144]].concat(),
    ] {
        assert!(bounded::check(&bytes).is_err());
    }
}

#[test]
fn rejects_invalid_async_syntax_sites() {
    let source = "await 1; await 2;";
    let original = record(source, "async.mjs", ModuleFormat::Esm);
    assert_eq!(original.module.facts.top_level_await.len(), 2);
    for mutate in [
        |r: &mut CacheRecord| r.module.facts.top_level_await.reverse(),
        |r: &mut CacheRecord| r.module.facts.top_level_await[0].end = u32::MAX,
        |r: &mut CacheRecord| r.module.facts.top_level_await[0] = Span::new(8, 8),
        |r: &mut CacheRecord| r.schema = 4,
        |r: &mut CacheRecord| r.extractor = 5,
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
}
