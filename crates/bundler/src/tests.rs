use super::*;
use ast_segmenter::{parse_module, ModuleFormat};
use source_graph::OwnedSourceFileInput;
use std::path::Path;

pub(crate) fn input(path: &str, source: &str) -> OwnedSourceFileInput {
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new(path),
        ModuleFormat::Esm,
    )
    .unwrap();
    let resolutions = parsed
        .facts
        .dependencies
        .iter()
        .map(|d| (d.id, DependencyTarget::File(d.specifier.clone().into())))
        .collect();
    OwnedSourceFileInput {
        path: path.into(),
        segments: parsed.segments,
        facts: parsed.facts,
        resolutions,
    }
}

fn retained_sources<'a>(
    graph: &SourceGraph,
    plan: &RetentionPlan,
    sources: &'a [&str],
) -> Vec<(u32, &'a str, Vec<RetentionReason>)> {
    plan.segments
        .iter()
        .map(|s| {
            let span = graph.file_segments(s.key.file_id).unwrap()[s.key.segment_idx as usize].span;
            (
                s.key.file_id,
                &sources[s.key.file_id as usize][span.start as usize..span.end as usize],
                s.reasons.clone(),
            )
        })
        .collect()
}

#[test]
fn runtime_imports_evaluate_unreferenced_exports_and_no_export_modules() {
    let sources = [
        "import {unused} from 'lib.ts'; import type {T} from 'types.ts';",
        "import 'effects.ts'; export const unused = initialize(helper); const helper = 1; function dead() { nested(); }",
        "first(); second();",
        "export type T = string; typeOnlyEffect();",
    ];
    let graph = SourceGraph::from_owned([
        input("entry.ts", sources[0]),
        input("lib.ts", sources[1]),
        input("effects.ts", sources[2]),
        input("types.ts", sources[3]),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0, 1, 2]);
    insta::assert_debug_snapshot!(retained_sources(&graph, &plan, &sources));
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 2)));
    assert!(!plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 3)));
    assert!(plan
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::EffectOrder
            && e.from == GraphNode::Segment(SegmentKey::new(2, 1))
            && e.to == GraphNode::Segment(SegmentKey::new(2, 0))));
    assert_eq!(plan, retain(&graph, &[0], &[]).unwrap());
}

#[test]
fn binding_demand_keeps_checked_barrel_routes_without_rooting_every_binding() {
    let sources = [
        "import {used} from 'barrel.ts'; consume(used);",
        "export {used} from 'leaf.ts';",
        "export const used = 1; export const dead = 2; initialize();",
    ];
    let graph = SourceGraph::from_owned([
        input("entry.ts", sources[0]),
        input("barrel.ts", sources[1]),
        input("leaf.ts", sources[2]),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(2, 0)));
    assert!(plan.segments.iter().any(|s| s.key == SegmentKey::new(1, 0)));
    assert!(!plan.segments.iter().any(|s| s.key == SegmentKey::new(2, 1)));
    let export = graph
        .resolve_export(1, "used", source_graph::Namespace::Value)
        .unwrap();
    let direct = retain(&graph, &[], &export.route).unwrap();
    assert_eq!(direct.modules, [1, 2]);
}

#[test]
fn cycles_and_empty_modules_preserve_identity_and_source_order() {
    let graph = SourceGraph::from_owned([
        input(
            "a.ts",
            "import 'b.ts'; import 'empty.ts'; first(); second();",
        ),
        input("b.ts", "import 'a.ts'; third();"),
        input("empty.ts", ""),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0, 1, 2]);
    assert_eq!(plan.segments.len(), 3);
    assert_eq!(
        plan.edges
            .iter()
            .filter(|e| e.kind == RetentionEdgeKind::StaticEvaluation)
            .count(),
        3
    );
    insta::assert_debug_snapshot!(plan);
}

#[test]
fn deferred_bodies_are_not_eager_effects_and_async_sites_are_boundaries() {
    let graph = SourceGraph::from_owned([
        input("entry.cjs", "function later() { return require('dep.cjs'); }"),
        input("dep.cjs", "run();"),
        input("esm.ts", "export function later() { return import('async.ts'); }"),
        input("async.ts", "asyncEffect();"),
        input("pure.ts", "function unused(x = unknown()) { hidden(); } const arrow = () => hidden(); class C { x = hidden(); }"),
    ]);
    let pure = retain(&graph, &[4], &[]).unwrap();
    assert!(pure.segments.is_empty());
    let exported = retain(&graph, &[], &[GraphNode::Segment(SegmentKey::new(2, 0))]).unwrap();
    assert_eq!(exported.modules, [2]);
    assert_eq!(exported.async_boundaries.len(), 1);
    let cjs = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(cjs.modules, [0, 1]);
    let require = cjs
        .edges
        .iter()
        .find(|e| e.kind == RetentionEdgeKind::Require)
        .unwrap();
    assert!(graph.edges()[require.source_edge.unwrap()]
        .enclosing_function
        .is_some());
    assert!(!cjs
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::StaticEvaluation));
}

#[test]
fn rejects_invalid_roots_legacy_inputs_and_link_errors() {
    let graph = SourceGraph::from_owned([input("empty.ts", "")]);
    assert!(retain(&graph, &[42], &[]).is_err());
    assert!(retain(&graph, &[], &[GraphNode::Segment(SegmentKey::new(0, 0))]).is_err());
    let legacy = SourceGraph::new(std::iter::once(source_graph::SourceFileInput {
        source_file_path: "legacy.ts".into(),
        segments: vec![],
        resolved_reexport_paths: Default::default(),
        resolved_import_paths: Default::default(),
    }));
    assert!(retain(&legacy, &[0], &[]).is_err());
    let broken = SourceGraph::from_owned([
        input("a.ts", "import {missing} from 'b.ts';"),
        input("b.ts", ""),
    ]);
    assert!(retain(&broken, &[0], &[]).is_err());
}

#[test]
fn namespace_demand_is_conservative_and_plans_expire_on_mutation() {
    let mut graph = SourceGraph::from_owned([
        input("entry.ts", "import * as ns from 'lib.ts'; consume(ns);"),
        input("lib.ts", "export const one = 1; export const two = 2;"),
    ]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(
        plan.segments.iter().filter(|s| s.key.file_id == 1).count(),
        2
    );
    assert!(plan.is_current(&graph, &policy::EffectPolicy::default()));
    graph
        .patch_owned(
            Path::new("lib.ts"),
            input("lib.ts", "export const one = 3;"),
        )
        .unwrap();
    assert!(!plan.is_current(&graph, &policy::EffectPolicy::default()));
}

#[test]
fn cache_ranges_and_facts_produce_the_same_plan_without_tracker_maps() {
    let sources = [
        ("entry.ts", "import {value} from 'lib.ts'; consume(value);"),
        ("lib.ts", "export const value = 1; initialize();"),
    ];
    let cold = SourceGraph::from_owned(sources.iter().map(|(p, s)| input(p, s)));
    let cached = SourceGraph::from_owned(sources.iter().map(|(path, source)| {
        let parsed = parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new(path),
            ModuleFormat::Esm,
        )
        .unwrap();
        let record = segment_cache::CacheRecord::from_parsed(source, &parsed).unwrap();
        let bytes = record.encode(source).unwrap();
        drop(parsed);
        let record = segment_cache::CacheRecord::decode(&bytes, source, record.config).unwrap();
        let resolutions = record
            .module
            .facts
            .dependencies
            .iter()
            .map(|d| (d.id, DependencyTarget::File(d.specifier.clone().into())))
            .collect();
        OwnedSourceFileInput::from_ranges(
            (*path).into(),
            record.module.segments,
            record.module.facts,
            resolutions,
        )
    }));
    assert_eq!(
        retain(&cold, &[0], &[]).unwrap(),
        retain(&cached, &[0], &[]).unwrap()
    );
}

#[test]
fn external_evaluation_is_explicit_and_type_reexports_do_not_evaluate() {
    let mut entry = input(
        "entry.ts",
        "import 'external'; export type {T} from 'types.ts';",
    );
    let dependency = entry.facts.dependencies[0].id;
    entry
        .resolutions
        .insert(dependency, DependencyTarget::External("external".into()));
    let graph =
        SourceGraph::from_owned([entry, input("types.ts", "export type T = string; run();")]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.modules, [0]);
    assert_eq!(plan.edges.len(), 1);
    assert!(matches!(plan.edges[0].to, GraphNode::Unlinked { .. }));
}
