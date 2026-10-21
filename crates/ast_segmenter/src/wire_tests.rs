use super::*;
use crate::{parse_module, ModuleFormat, ParsedModule};
use std::path::Path;

fn encoded(source: &str, path: &str) -> (ParsedModule, cache_flatbuffers::WireRecord) {
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        ModuleFormat::Esm,
    )
    .unwrap();
    let bytes = parsed.to_flatbuffer(source, "test-oxc").unwrap();
    assert_eq!(bytes, parsed.to_flatbuffer(source, "test-oxc").unwrap());
    let wire = cache_flatbuffers::WireRecord::new(
        bytes.into(),
        source,
        parsed.provenance().config().wire(),
        "test-oxc",
    )
    .unwrap();
    (parsed, wire)
}

fn equal(a: ModuleFacts<'_>, b: ModuleFacts<'_>) {
    macro_rules! fields {
        ($($name:ident),+ $(,)?) => { $(assert_eq!(a.$name(), b.$name(), stringify!($name));)+ };
    }
    fields!(
        ecco_declarations_hash,
        commonjs,
        top_level_await,
        effects,
        invocations,
        emission,
        bindings,
        references,
        dependencies,
        imports,
        exports,
        directives,
        hashbang
    );
    match (a.ecco(), b.ecco()) {
        (Some(a), Some(b)) => {
            a.validate().unwrap();
            b.validate().unwrap();
            assert_eq!(a.values().len(), b.values().len());
            assert_eq!(a.functions().len(), b.functions().len());
            assert_eq!(
                a.roots().iter().map(|r| r.span()).collect::<Vec<_>>(),
                b.roots().iter().map(|r| r.span()).collect::<Vec<_>>()
            );
            assert_eq!(
                a.symbols()
                    .iter()
                    .map(|s| (s.id(), s.name(), s.span()))
                    .collect::<Vec<_>>(),
                b.symbols()
                    .iter()
                    .map(|s| (s.id(), s.name(), s.span()))
                    .collect::<Vec<_>>()
            );
        }
        (None, None) => {}
        _ => panic!("Ecco representation changed"),
    }
}

#[test]
fn owned_and_wire_facts_are_lossless_after_arena_and_owner_drop() {
    for (path, source) in [
        ("input.mjs", "#!/bin/node\n'use strict'; import def, {x as alias} from './a'; import * as ns from './b'; export const café = alias, next = ns.value; export {café as 'é'}; export * from './c'; export * as space from './d'; export default function(){ return def; }"),
        ("input.mjs", "let x = 0; const stored = (p = x++) => x++; (() => x++)(); class C { [x++] = x++; static value = x++; static { x++; } method(p = x++) { return () => x++; } }"),
        ("input.mjs", "/*#__PURE__*/ object.method().next?.(...iterable); /*#__PURE__*/ (object?.method)(await argument); async function deferred(){ /*#__PURE__*/ obj?.[key()](/*#__PURE__*/ inner(arg), ...iterable, await later); }"),
        ("input.mjs", "import * as ns from './ns'; ns.method(); ns.tag`x`; ns.value = 1; delete ns.value; consume(ns); ns['computed']; let __proto__, x; const obj = {__proto__, x}; ({__proto__, x = 1} = obj);"),
        ("input.mjs", "export const load = name => import(`./parts/${name}.mjs`); export default class Named {};"),
        ("input.mjs", "export default class { method() {} }; for await (const x of [1]) { await x; }"),
        ("input.cjs", "#!/bin/node\n'use strict'; module.exports = require('./dep'); function later(name) { return require('./parts/' + name); } exports.value = 1;"),
        ("empty.cjs", ""),
        ("zero-id.mjs", "x => x;"),
        ("async-views.mjs", "const target = 1; async function load(value = target) { target; (() => target)(); (async () => target)(); const ns = await import('./child.mjs'); ns.value; ns.method(); return this; }"),
        ("deferred-views.mjs", "export function ready() {} export let value = ready(); export let empty; console.log(value);"),
        ("input.ts", "import type { X } from './types'; export interface A { value: X }"),
        ("input.d.ts", "export interface A { value: string }"),
        ("input.jsx", "export const element = 1;"),
    ] {
        let (parsed, record) = encoded(source, path);
        equal(parsed.facts(), record.module().facts().into());
        let borrowed = ParsedModule::from_wire(source, record.clone()).unwrap();
        assert_eq!(borrowed.provenance().config(), parsed.provenance().config());
        let ranges = borrowed.segments().iter().map(|s| s.span).collect::<Vec<_>>();
        assert!(parsed.provenance().verify(source, parsed.provenance().config(), parsed.body(), &ranges, borrowed.facts()).is_err());
        assert!(borrowed.provenance().verify(source, borrowed.provenance().config(), borrowed.body(), &ranges, parsed.facts()).is_err());
        let expected = parsed.owned_facts().unwrap().clone();
        drop(parsed);
        drop(record);
        equal((&expected).into(), borrowed.facts());
        borrowed.provenance().verify(source, borrowed.provenance().config(), borrowed.body(), &ranges, borrowed.facts()).unwrap();
        assert!(borrowed.owned_facts().is_err());
        assert!(borrowed.owned_segments().is_none());
        assert!(borrowed.provenance().verify(&format!("{source} "), borrowed.provenance().config(), borrowed.body(), &ranges, borrowed.facts()).is_err());
        let mut wrong_ranges = ranges.clone();
        wrong_ranges.push(Span::new(0, 0));
        assert!(borrowed.provenance().verify(source, borrowed.provenance().config(), borrowed.body(), &wrong_ranges, borrowed.facts()).is_err());
    }
}

#[test]
fn names_nested_tables_and_ecco_borrow_the_verified_allocation() {
    let source = "import {value as local} from './dep'; export const result = local; function closure(){ return local; }";
    let (_, record) = encoded(source, "input.mjs");
    let start = record.bytes().as_ptr() as usize;
    let end = start + record.bytes().len();
    let facts = ModuleFacts::Wire(record.module().facts());
    for text in facts
        .bindings()
        .iter()
        .map(|v| v.name())
        .chain(facts.references().iter().map(|v| v.name()))
        .chain(facts.dependencies().iter().map(|v| v.specifier()))
        .chain(facts.ecco().unwrap().symbols().iter().map(|v| v.name()))
    {
        assert!(start <= text.as_ptr() as usize && text.as_ptr() as usize + text.len() <= end);
    }
    assert!(matches!(facts.emission(), EmissionFacts::Wire(_)));
    assert!(matches!(
        facts.emission().execution(),
        ExecutionFacts::Wire(_)
    ));
    assert!(matches!(
        facts.emission().segments().get(0).unwrap(),
        SegmentRecipe::Wire(_)
    ));
    assert!(matches!(
        facts.ecco().unwrap(),
        ecco::views::ParsedFileView::Wire(_)
    ));
}
