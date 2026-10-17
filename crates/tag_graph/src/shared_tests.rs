use super::*;
use ast_segmenter::{parse_module, ModuleFormat};
use source_graph::{DependencyTarget, OwnedSourceFileInput, SourceFileInput};
use std::path::Path;

fn input(path: &str, source: &str) -> OwnedSourceFileInput {
    input_with_format(path, source, ModuleFormat::Esm)
}

fn input_with_format(path: &str, source: &str, format: ModuleFormat) -> OwnedSourceFileInput {
    let parsed =
        parse_module(&logger::StdioLogger::new(), source, Path::new(path), format).unwrap();
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

fn graphs() -> (SourceGraph, SourceGraph) {
    graph_pair(vec![
        input("entry.ts", "import {y} from 'barrel.ts'; import type {T} from 'barrel.ts'; import('empty.ts'); console.log(y);"),
        input("barrel.ts", "export {x as y} from 'leaf.ts'; export type {T} from 'leaf.ts';"),
        input("leaf.ts", "export const x = 1; export type T = number; export const dead = 2;"),
        input("empty.ts", ""),
    ])
}

fn graph_pair(inputs: Vec<OwnedSourceFileInput>) -> (SourceGraph, SourceGraph) {
    let legacy = SourceGraph::new(inputs.iter().map(|i| {
        SourceFileInput {
            source_file_path: i.path.clone(),
            segments: i.segments.clone(),
            resolved_import_paths: i
                .facts
                .dependencies
                .iter()
                .map(|d| (d.specifier.clone(), d.specifier.clone().into()))
                .collect(),
            resolved_reexport_paths: i
                .facts
                .dependencies
                .iter()
                .map(|d| (d.specifier.clone(), d.specifier.clone().into()))
                .collect(),
        }
    }));
    (legacy, SourceGraph::from_owned(inputs))
}

#[test]
fn owned_and_legacy_share_usage_policy_barrels_types_and_reverse_edges() {
    let (legacy, owned) = graphs();
    for follow in [false, true] {
        for tag in [
            UsedTag::FROM_ENTRY,
            UsedTag::FROM_TEST,
            UsedTag::FROM_IGNORED,
        ] {
            let mut results = Vec::new();
            for graph in [&legacy, &owned] {
                let mut tags = TagGraph::new();
                tags.propagate_tags_to_used(
                    graph,
                    vec![
                        SegmentKey::new(0, 0),
                        SegmentKey::new(0, 1),
                        SegmentKey::new(0, 2),
                    ],
                    tag,
                    follow,
                );
                assert_eq!(tags.get_tag(SegmentKey::new(1, 0)), tag);
                assert_eq!(tags.get_tag(SegmentKey::new(2, 0)), tag);
                assert_eq!(
                    tags.get_tag(SegmentKey::new(2, 1)),
                    if follow { tag } else { UsedTag::empty() }
                );
                assert_eq!(tags.get_tag(SegmentKey::new(2, 2)), UsedTag::empty());
                assert_eq!(tags.file_tag(graph, 3), tag);
                results.push((tags.tags, tags.file_tags));

                let mut reverse = TagGraph::new();
                reverse.propagate_tags_to_users(graph, vec![SegmentKey::new(2, 0)], tag, follow);
                assert_eq!(reverse.get_tag(SegmentKey::new(1, 0)), tag);
                assert_eq!(reverse.get_tag(SegmentKey::new(0, 0)), tag);
                let mut reverse_dead = TagGraph::new();
                reverse_dead.propagate_tags_to_users(
                    graph,
                    vec![SegmentKey::new(2, 2)],
                    tag,
                    follow,
                );
                assert_eq!(
                    reverse_dead.get_tag(SegmentKey::new(0, 0)),
                    UsedTag::empty()
                );
                let mut reverse_type = TagGraph::new();
                reverse_type.propagate_tags_to_users(
                    graph,
                    vec![SegmentKey::new(2, 1)],
                    tag,
                    follow,
                );
                assert_eq!(
                    reverse_type.get_tag(SegmentKey::new(0, 1)),
                    if follow { tag } else { UsedTag::empty() }
                );
            }
            assert_eq!(results[0], results[1]);
        }
    }
}

#[test]
fn namespace_execution_and_async_usage_share_both_indexes() {
    for statement in [
        "import * as ns from 'target.ts';",
        "export * from 'target.ts';",
        "export * as ns from 'target.ts';",
        "import('target.ts');",
        "require('target.ts');",
        "import 'target.ts';",
    ] {
        let format = if statement.starts_with("require(") {
            ModuleFormat::CommonJs
        } else {
            ModuleFormat::Esm
        };
        let (legacy, owned) = graph_pair(vec![
            input_with_format("entry.ts", statement, format),
            input(
                "target.ts",
                "export const live = 1; export const other = 2;",
            ),
        ]);
        for graph in [&legacy, &owned] {
            let mut tags = TagGraph::new();
            tags.propagate_tags_to_used(
                graph,
                vec![SegmentKey::new(0, 0)],
                UsedTag::FROM_TEST,
                false,
            );
            for index in 0..2 {
                assert_eq!(
                    tags.get_tag(SegmentKey::new(1, index)),
                    UsedTag::FROM_TEST,
                    "{statement}"
                );
                let mut reverse = TagGraph::new();
                reverse.propagate_tags_to_users(
                    graph,
                    vec![SegmentKey::new(1, index)],
                    UsedTag::FROM_IGNORED,
                    false,
                );
                assert_eq!(
                    reverse.get_tag(SegmentKey::new(0, 0)),
                    UsedTag::FROM_IGNORED,
                    "{statement}"
                );
                assert!(reverse.get_tag(SegmentKey::new(1, 1 - index)).is_empty());
            }
        }
    }
}

#[test]
fn legacy_external_side_effect_statement_keeps_its_usage_tag() {
    let parsed = input(
        "entry.ts",
        "import 'external'; export const live = 1; export const dead = 2;",
    );
    let graph = SourceGraph::new(std::iter::once(SourceFileInput {
        source_file_path: parsed.path,
        segments: parsed.segments,
        resolved_import_paths: Default::default(),
        resolved_reexport_paths: Default::default(),
    }));
    let mut tags = TagGraph::new();
    tags.propagate_tags_to_used(
        &graph,
        vec![SegmentKey::new(0, 1)],
        UsedTag::FROM_ENTRY,
        false,
    );
    assert_eq!(tags.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
    assert!(tags.get_tag(SegmentKey::new(0, 2)).is_empty());
    graph.validate_links().unwrap();
}

#[test]
fn namespace_reexport_reaches_empty_modules_and_missing_names_do_not_fallback() {
    let graph = SourceGraph::from_owned([
        input("entry.ts", "import {ns} from 'barrel.ts';"),
        input("barrel.ts", "export * as ns from 'empty.ts';"),
        input("empty.ts", ""),
    ]);
    graph.validate_links().unwrap();
    let mut tags = TagGraph::new();
    tags.propagate_tags_to_used(
        &graph,
        vec![SegmentKey::new(0, 0)],
        UsedTag::FROM_ENTRY,
        false,
    );
    assert_eq!(tags.file_tag(&graph, 2), UsedTag::FROM_ENTRY);
    let graph = SourceGraph::from_owned([
        input("entry.ts", "import {missing} from 'target.ts';"),
        input("target.ts", "export const unrelated = 1;"),
    ]);
    assert!(graph.validate_links().is_err());
    let mut tags = TagGraph::new();
    tags.propagate_tags_to_used(
        &graph,
        vec![SegmentKey::new(0, 0)],
        UsedTag::FROM_ENTRY,
        false,
    );
    assert_eq!(tags.file_tag(&graph, 1), UsedTag::empty());
}

#[test]
fn structural_revisions_discard_old_usage_state() {
    let mut graph = SourceGraph::from_owned([input("entry.ts", "export const old = 1;")]);
    let mut tags = TagGraph::new();
    tags.propagate_tags_to_used(
        &graph,
        vec![SegmentKey::new(0, 0)],
        UsedTag::FROM_ENTRY,
        false,
    );
    graph
        .patch_owned(
            Path::new("entry.ts"),
            input("entry.ts", "export const replacement = 2;"),
        )
        .unwrap();
    assert!(tags.file_tag(&graph, 0).is_empty());
    tags.propagate_tags_to_used(&graph, vec![], UsedTag::FROM_TEST, false);
    assert!(tags.get_tag(SegmentKey::new(0, 0)).is_empty());
}
