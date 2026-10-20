use crate::{structure::tests::input, *};
use import_resolver::{
    node_resolver::caching::NodeModulesResolverOptions, resolve::TargetEnv,
    structural::StructuralResolver,
};
use std::{collections::BTreeSet, fs, path::Path};
use test_tmpdir::test_tmpdir;

fn normalized(graph: &SourceGraph) -> Vec<String> {
    fn node(graph: &SourceGraph, node: GraphNode) -> String {
        match node {
            GraphNode::Module(id) => format!("{}:module", graph.file_path(id).unwrap().display()),
            GraphNode::Segment(k) => format!(
                "{}:{}",
                graph.file_path(k.file_id).unwrap().display(),
                k.segment_idx
            ),
            GraphNode::Unlinked {
                file_id,
                dependency,
            } => format!(
                "{}:unlinked:{}",
                graph.file_path(file_id).unwrap().display(),
                dependency.0
            ),
        }
    }
    let mut edges: Vec<_> = graph
        .edges()
        .iter()
        .map(|e| {
            format!(
                "{} -> {} {:?} {:?} {:?}",
                node(graph, e.from),
                node(graph, e.to),
                e.kind,
                e.site,
                e.imported
            )
        })
        .collect();
    edges.sort();
    edges
}

fn legacy_input(path: &str, source: &str) -> SourceFileInput {
    SourceFileInput {
        source_file_path: path.into(),
        segments: input(path, source).segments,
        resolved_import_paths: Default::default(),
        resolved_reexport_paths: Default::default(),
    }
}

fn assert_dead_id(graph: &SourceGraph, id: u32) {
    assert!(graph.file_path(id).is_none());
    assert!(graph.file_segments(id).is_none());
    assert!(graph.file_symbol_to_segment(id).is_none());
    assert!(graph.file_resolved_import_paths(id).is_none());
    assert!(graph.file_facts(id).is_none());
    assert!(graph.file_provenance(id).is_none());
    assert!(graph.file_prepared_resource(id).is_none());
    assert!(graph
        .dependency_target(id, ast_segmenter::owned::DependencyId(0))
        .is_none());
    assert!(graph
        .resolve_binding(id, ast_segmenter::owned::BindingId(0))
        .is_none());
    assert!(graph.resolve_symbol_in_file(id, "value", 0).is_none());
    assert!(graph.resolve_export(id, "value", Namespace::Value).is_err());
    assert!(graph
        .resolve_optional_export(id, "value", Namespace::Value)
        .unwrap()
        .is_none());
    assert!(graph
        .resolve_import_across_files(id, &ExportedSymbol::from("value"))
        .is_empty());
    assert!(!graph.module_ids().any(|live| live == id));
    assert!(!graph.iter_segments().any(|(key, _)| key.file_id == id));
    for node in [
        GraphNode::Module(id),
        GraphNode::Segment(SegmentKey::new(id, 0)),
    ] {
        assert!(graph.incoming(node).next().is_none());
        assert!(graph.outgoing(node).next().is_none());
        assert!(graph.outgoing_indexed(node).next().is_none());
    }
}

#[test]
fn checked_ids_stay_dead_across_owned_and_legacy_recreation() {
    for owned in [false, true] {
        let mut graph = if owned {
            SourceGraph::from_owned([
                input("input.mjs", "export const value = 1;"),
                input("empty.mjs", ""),
            ])
        } else {
            SourceGraph::new(
                [
                    legacy_input("input.mjs", "export const value = 1;"),
                    legacy_input("empty.mjs", ""),
                ]
                .into_iter(),
            )
        };
        let path = Path::new("input.mjs");
        let empty_id = graph.file_id(Path::new("empty.mjs")).unwrap();
        let mut retired = Vec::new();
        for patch_owned in [true, false, true, false] {
            let id = graph.file_id(path).unwrap();
            assert_eq!(graph.file_count(), 2);
            assert_eq!(graph.module_ids().count(), 2);
            assert_eq!(graph.file_segments(empty_id).unwrap().len(), 0);
            assert!(graph.resolve_export(id, "value", Namespace::Value).is_ok());
            assert!(!graph
                .resolve_import_across_files(id, &ExportedSymbol::from("value"))
                .is_empty());
            let old_node = graph.versioned_node(GraphNode::Module(id));
            let patch = |graph: &mut SourceGraph| {
                if patch_owned {
                    graph.patch_owned(path, input("input.mjs", "export const value = 2;"))
                } else {
                    graph.patch_file(path, legacy_input("input.mjs", "export const value = 2;"))
                }
                .unwrap();
            };
            patch(&mut graph);
            assert_eq!(graph.file_id(path), Some(id));
            assert!(!graph.is_current(old_node));
            assert!(graph.remove_file(path));
            retired.push(id);
            assert_eq!(graph.file_count(), 1);
            assert_eq!(graph.module_ids().collect::<Vec<_>>(), [empty_id]);
            assert!(graph.file_id(path).is_none());
            for &dead in &retired {
                assert_dead_id(&graph, dead);
            }
            let revision = graph.revision();
            assert!(!graph.remove_file(path));
            assert_eq!(graph.revision(), revision);
            patch(&mut graph);
            assert!(graph.file_id(path).unwrap() > id.max(empty_id));
            for &dead in &retired {
                assert_dead_id(&graph, dead);
            }
            assert_dead_id(&graph, u32::MAX);
        }
        assert_eq!(graph.file_count(), 2);
        assert_eq!(graph.module_ids().count(), 2);
    }
}

#[test]
fn duplicate_construction_paths_keep_only_the_last_id_live() {
    for owned in [false, true] {
        let mut graph = if owned {
            SourceGraph::from_owned([
                input("input.mjs", "export const value = 1; console.log(value);"),
                input("input.mjs", ""),
            ])
        } else {
            SourceGraph::new(
                [
                    legacy_input("input.mjs", "export const value = 1; console.log(value);"),
                    legacy_input("input.mjs", ""),
                ]
                .into_iter(),
            )
        };
        assert_eq!(graph.file_id(Path::new("input.mjs")), Some(1));
        assert_eq!(graph.file_count(), 1);
        assert_eq!(graph.module_ids().collect::<Vec<_>>(), [1]);
        assert_eq!(graph.file_segments(1).unwrap().len(), 0);
        assert_dead_id(&graph, 0);
        assert!(graph.remove_file(Path::new("input.mjs")));
        assert_eq!(graph.file_count(), 0);
        assert_eq!(graph.module_ids().count(), 0);
        assert_dead_id(&graph, 0);
        assert_dead_id(&graph, 1);
    }
}

#[test]
fn source_patch_replaces_provenance_and_invalidates_existing_handles() {
    let mut graph = SourceGraph::from_owned([input("input.mjs", "export const value = 1;")]);
    let old = graph.file_provenance(0).unwrap().clone();
    let node = graph.versioned_node(GraphNode::Module(0));
    graph
        .patch_owned(
            Path::new("input.mjs"),
            input("input.mjs", "export const value = 2;"),
        )
        .unwrap();
    assert!(!graph.is_current(node));
    assert_ne!(graph.file_provenance(0), Some(&old));
    let source_revision = graph.revision().source;
    assert!(graph.remove_file(Path::new("input.mjs")));
    assert!(graph.file_provenance(0).is_none());
    assert!(graph.revision().source > source_revision);
}

#[test]
fn missing_added_removed_recreated_matches_rebuild_and_rejects_rename() {
    let dir = test_tmpdir! { "entry.ts" => "import {x} from './target'; console.log(x);" };
    let entry = dir.root().join("entry.ts");
    let target = dir.root().join("target.ts");
    let source = fs::read_to_string(&entry).unwrap();
    let mut resolver = StructuralResolver::new(
        dir.root().into(),
        NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        BTreeSet::new(),
    );
    let mut graph = SourceGraph::from_owned([input(entry.to_str().unwrap(), &source)]);
    assert!(graph.refresh_resolution(&resolver).is_err());
    assert!(graph
        .diagnostics()
        .iter()
        .any(|d| d.message.contains("target")));
    fs::write(&target, "export const x = 1;").unwrap();
    graph
        .patch_owned(
            &target,
            input(target.to_str().unwrap(), "export const x = 1;"),
        )
        .unwrap();
    resolver.invalidate();
    graph.refresh_resolution(&resolver).unwrap();
    let old_id = graph.file_id(&target).unwrap();
    let old_node = graph.versioned_node(GraphNode::Segment(SegmentKey::new(old_id, 0)));
    assert!(graph.incoming(old_node.node).next().is_some());
    let revision = graph.revision();
    assert!(graph.patch_owned(&target, input("renamed.ts", "")).is_err());
    assert!(graph
        .patch_file(
            &target,
            SourceFileInput {
                source_file_path: "renamed.ts".into(),
                segments: vec![],
                resolved_import_paths: Default::default(),
                resolved_reexport_paths: Default::default(),
            }
        )
        .is_err());
    assert_eq!(graph.revision(), revision);
    fs::remove_file(&target).unwrap();
    assert!(graph.remove_file(&target));
    assert!(!graph.is_current(old_node));
    assert!(graph.file_path(old_id).is_none());
    assert!(graph.incoming(old_node.node).next().is_none());
    resolver.invalidate();
    assert!(graph.refresh_resolution(&resolver).is_err());
    fs::write(&target, "export const x = 2;").unwrap();
    graph
        .patch_owned(
            &target,
            input(target.to_str().unwrap(), "export const x = 2;"),
        )
        .unwrap();
    assert_ne!(graph.file_id(&target), Some(old_id));
    resolver.invalidate();
    graph.refresh_resolution(&resolver).unwrap();
    let mut full = SourceGraph::from_owned([
        input(entry.to_str().unwrap(), &source),
        input(target.to_str().unwrap(), "export const x = 2;"),
    ]);
    full.refresh_resolution(&resolver).unwrap();
    assert_eq!(normalized(&graph), normalized(&full));
    assert_eq!(graph.diagnostics(), full.diagnostics());
    for edge in graph.edges() {
        assert!(graph.incoming(edge.to).any(|e| std::ptr::eq(e, edge)));
    }
}

#[test]
fn conditions_package_and_tsconfig_edits_relink_without_source_change() {
    let dir = test_tmpdir! {
        "entry.cjs" => "require('pkg'); import('pkg'); require('alias');",
        "node_modules/pkg/package.json" => r#"{"name":"pkg","exports":{".":{"import":"./i.js","require":"./r.js"}}}"#,
        "node_modules/pkg/i.js" => "",
        "node_modules/pkg/r.js" => "",
        "node_modules/alias/package.json" => r#"{"exports":{".":{"two":"./two.js","default":"./one.js"}}}"#,
        "node_modules/alias/one.js" => "",
        "node_modules/alias/two.js" => "",
        "tsconfig.json" => r#"{"compilerOptions":{"customConditions":[]}}"#
    };
    let entry = dir.root().join("entry.cjs");
    let source = fs::read_to_string(&entry).unwrap();
    let make = || {
        let parsed = ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            &source,
            &entry,
            ast_segmenter::ModuleFormat::CommonJs,
        )
        .unwrap();
        let mut inputs = vec![OwnedSourceFileInput::from_parsed(
            entry.clone(),
            parsed,
            Default::default(),
        )];
        for path in [
            "node_modules/pkg/i.js",
            "node_modules/pkg/r.js",
            "node_modules/alias/one.js",
            "node_modules/alias/two.js",
        ] {
            inputs.push(input(dir.root().join(path).to_str().unwrap(), ""));
        }
        SourceGraph::from_owned(inputs)
    };
    let mut graph = make();
    let mut resolver = StructuralResolver::new(
        dir.root().into(),
        NodeModulesResolverOptions::default_for_env(TargetEnv::Node),
        BTreeSet::new(),
    );
    graph.refresh_resolution(&resolver).unwrap();
    let targets = |g: &SourceGraph| {
        g.edges()
            .iter()
            .filter(|e| e.dependency.is_some())
            .map(|e| match e.to {
                GraphNode::Module(id) => g
                    .file_path(id)
                    .unwrap()
                    .strip_prefix(dir.root())
                    .unwrap()
                    .to_owned(),
                _ => panic!("resolved module"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        targets(&graph),
        [
            "node_modules/pkg/r.js",
            "node_modules/pkg/i.js",
            "node_modules/alias/one.js"
        ]
        .map(std::path::PathBuf::from)
    );
    let before = graph.revision();
    fs::write(
        dir.root().join("tsconfig.json"),
        r#"{"compilerOptions":{"customConditions":["two"]}}"#,
    )
    .unwrap();
    fs::write(
        dir.root().join("node_modules/pkg/package.json"),
        r#"{"name":"pkg","exports":{".":{"import":"./r.js","require":"./i.js"}}}"#,
    )
    .unwrap();
    resolver.invalidate();
    graph.refresh_resolution(&resolver).unwrap();
    assert_eq!(graph.revision().source, before.source);
    assert!(graph.revision().resolution > before.resolution);
    assert_eq!(
        targets(&graph),
        [
            "node_modules/pkg/i.js",
            "node_modules/pkg/r.js",
            "node_modules/alias/two.js"
        ]
        .map(std::path::PathBuf::from)
    );
    let mut full = make();
    full.refresh_resolution(&resolver).unwrap();
    assert_eq!(normalized(&graph), normalized(&full));
    assert!(resolver.revision() > 0);
}

#[test]
fn external_policy_is_explicit_and_preserves_sites() {
    let dir = test_tmpdir! { "entry.ts" => "import 'node:fs'; import 'declared';" };
    let entry = dir.root().join("entry.ts");
    let mut graph = SourceGraph::from_owned([input(
        entry.to_str().unwrap(),
        "import 'node:fs'; import 'declared'; export {x} from 'declared';",
    )]);
    let options = || NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
    let mut resolver = StructuralResolver::new(dir.root().into(), options(), BTreeSet::new());
    assert!(graph.refresh_resolution(&resolver).is_err());
    resolver.set_policy(
        options(),
        ["node:fs".to_owned(), "declared".to_owned()].into(),
    );
    graph.refresh_resolution(&resolver).unwrap();
    assert_eq!(graph.edges().len(), 4);
    assert_eq!(
        graph
            .resolve_export(0, "x", Namespace::Value)
            .unwrap()
            .identity,
        ExportIdentity::External("declared".to_owned(), Some("x".to_owned()))
    );
    for edge in graph.edges() {
        assert!(matches!(edge.to, GraphNode::Unlinked { .. }));
        assert!(matches!(
            graph.dependency_target(0, edge.dependency.unwrap()),
            Some(DependencyTarget::External(_))
        ));
    }
    assert!(!graph.remove_file(Path::new("absent")));
}

#[cfg(unix)]
#[test]
fn symlink_policy_is_explicit() {
    use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};
    let dir = test_tmpdir! { "entry.ts" => "", "real.ts" => "" };
    let alias = dir.root().join("alias.ts");
    std::os::unix::fs::symlink(dir.root().join("real.ts"), &alias).unwrap();
    for preserve in [false, true] {
        let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
        options.preserve_symlinks = preserve;
        let resolver = StructuralResolver::new(dir.root().into(), options, BTreeSet::new());
        assert_eq!(
            resolver
                .resolve_dependency(
                    &dir.root().join("entry.ts"),
                    "./alias",
                    ResolutionMode::Import
                )
                .unwrap(),
            ResolvedDependency::File(if preserve {
                alias.clone()
            } else {
                dir.root().join("real.ts")
            })
        );
    }
}
