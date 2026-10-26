use crate::structure::tests::input;
use crate::*;
use ast_segmenter::ExportedSymbol;

#[test]
fn unknown_external_star_collisions_keep_native_boundaries_and_all_routes() {
    let mut barrel = input("barrel.mjs", "export * from 'one'; export * from 'two';");
    for dependency in barrel.facts.dependencies() {
        barrel.resolutions.insert(
            dependency.id(),
            DependencyTarget::External(dependency.specifier().to_owned()),
        );
    }
    let graph = SourceGraph::from_owned([
        barrel,
        input(
            "main.mjs",
            "import {value} from 'barrel.mjs'; export {value};",
        ),
    ]);
    graph.validate_links().unwrap();
    assert!(graph.has_external_star_exports(0));
    assert!(!graph.has_external_star_exports(1));
    let linked = graph.resolve_export(0, "value", Namespace::Value).unwrap();
    assert_eq!(linked.identity, ExportIdentity::Native(0, "value".into()));
    for segment in 0..2 {
        assert!(linked
            .route
            .contains(&GraphNode::Segment(SegmentKey::new(0, segment))));
    }
    assert!(graph
        .resolve_export(0, "default", Namespace::Value)
        .is_err());
}

#[test]
fn lexical_forward_var_tdz_shadowing_and_namespaces() {
    let graph = SourceGraph::from_owned([input("a.ts",
        "later; variable; let later = 1; var variable; function f() { let later = 2; return later; } export type T = number; export { later };")]);
    assert_eq!(
        graph.resolve_symbol_in_file(0, "later", 0),
        Some(SegmentKey::new(0, 2))
    );
    assert_eq!(
        graph.resolve_symbol_in_file(0, "variable", 1),
        Some(SegmentKey::new(0, 3))
    );
    assert_eq!(
        graph.resolve_symbol_in_file(0, "later", 4),
        Some(SegmentKey::new(0, 4))
    );
    assert!(graph.resolve_export(0, "T", Namespace::Value).is_err());
    assert!(graph.resolve_export(0, "T", Namespace::Type).is_ok());
    assert_eq!(
        graph.resolve_import_across_files(0, &ExportedSymbol::from("later")),
        vec![SegmentKey::new(0, 2)]
    );
}

#[test]
fn stars_diamonds_explicit_precedence_namespace_alias_and_cycles() {
    let graph = SourceGraph::from_owned([
        input("leaf.ts", "export const x = 1; export default 2;"),
        input("left.ts", "export * from 'leaf.ts';"),
        input("right.ts", "export {x} from 'leaf.ts';"),
        input("diamond.ts", "export * from 'left.ts'; export * from 'right.ts';"),
        input("other.ts", "export const x = 3;"),
        input("ambiguous.ts", "export * from 'leaf.ts'; export * from 'other.ts';"),
        input("explicit.ts", "export * from 'ambiguous.ts'; export {x as x} from 'leaf.ts'; export * as ns from 'leaf.ts';"),
        input("cycle-a.ts", "export * from 'cycle-b.ts';"),
        input("cycle-b.ts", "export * from 'cycle-a.ts'; export * from 'leaf.ts';"),
        input("broken.ts", "export {missing as x} from 'leaf.ts'; export * from 'other.ts';"),
        input("alias.ts", "import {x as local} from 'leaf.ts'; export {local as alias};"),
    ]);
    let leaf = graph.resolve_export(0, "x", Namespace::Value).unwrap();
    let diamond = graph.resolve_export(3, "x", Namespace::Value).unwrap();
    assert_eq!(leaf.identity, diamond.identity);
    assert!(diamond
        .route
        .contains(&GraphNode::Segment(SegmentKey::new(1, 0))));
    assert!(diamond
        .route
        .contains(&GraphNode::Segment(SegmentKey::new(2, 0))));
    assert!(graph
        .resolve_export(3, "default", Namespace::Value)
        .is_err());
    assert!(graph
        .resolve_export(5, "x", Namespace::Value)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(graph
        .resolve_optional_export(5, "x", Namespace::Value)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert_eq!(
        graph
            .resolve_optional_export(0, "missing", Namespace::Value)
            .unwrap(),
        None
    );
    assert_eq!(
        graph
            .resolve_export(6, "x", Namespace::Value)
            .unwrap()
            .identity,
        leaf.identity
    );
    assert_eq!(
        graph
            .resolve_export(6, "ns", Namespace::Value)
            .unwrap()
            .identity,
        ExportIdentity::Namespace(0)
    );
    assert_eq!(
        graph
            .resolve_export(7, "x", Namespace::Value)
            .unwrap()
            .identity,
        leaf.identity
    );
    assert!(graph
        .resolve_export(7, "missing", Namespace::Value)
        .is_err());
    assert!(graph.resolve_export(9, "x", Namespace::Value).is_err());
    assert_eq!(
        graph
            .resolve_export(10, "alias", Namespace::Value)
            .unwrap()
            .identity,
        leaf.identity
    );
}

#[test]
fn link_edges_include_barrel_and_missing_ambiguous_diagnostics() {
    let graph = SourceGraph::from_owned([
        input("leaf.ts", "export const x = 1;"),
        input("barrel.ts", "export {x as y} from 'leaf.ts';"),
        input("entry.ts", "import {y} from 'barrel.ts'; console.log(y);"),
    ]);
    graph.validate_links().unwrap();
    let edges: Vec<_> = graph
        .outgoing(GraphNode::Segment(SegmentKey::new(2, 0)))
        .filter(|e| e.purpose == EdgePurpose::Binding)
        .map(|e| e.to)
        .collect();
    assert!(edges.contains(&GraphNode::Segment(SegmentKey::new(1, 0))));
    assert!(edges.contains(&GraphNode::Segment(SegmentKey::new(0, 0))));
    let graph = SourceGraph::from_owned([
        input("leaf.ts", "export const x = 1;"),
        input("entry.ts", "import {missing} from 'leaf.ts';"),
    ]);
    assert!(graph.validate_links().is_err());
    assert!(graph.diagnostics()[0].message.contains("missing"));
}

#[test]
fn decoded_cache_ranges_and_facts_link_without_legacy_maps() {
    use ast_segmenter::{parse_module, ModuleFormat};
    use segment_cache::{CacheRecord, ParserConfig};
    use std::path::Path;
    let source = "before; const before = 1; export {before as x};";
    let parsed = parse_module(
        &logger::StdioLogger::new(),
        source,
        Path::new("leaf.ts"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let cold = SourceGraph::from_owned([input("leaf.ts", source)]);
    let record = CacheRecord::from_parsed(source, &parsed).unwrap();
    let bytes = record.encode(source).unwrap();
    drop(parsed);
    let record = CacheRecord::decode(
        &bytes,
        source,
        ParserConfig::for_path(Path::new("leaf.ts"), ModuleFormat::Esm).unwrap(),
    )
    .unwrap();
    let warm = SourceGraph::from_owned([OwnedSourceFileInput::from_parsed(
        "leaf.ts".into(),
        record.to_parsed(source).unwrap(),
        Default::default(),
    )]);
    assert_eq!(cold.edges(), warm.edges());
    assert_eq!(
        cold.file_symbol_to_segment(0),
        warm.file_symbol_to_segment(0)
    );
    assert_eq!(
        cold.resolve_export(0, "x", Namespace::Value).unwrap(),
        warm.resolve_export(0, "x", Namespace::Value).unwrap()
    );
    assert_eq!(
        warm.resolve_symbol_in_file(0, "before", 0),
        Some(SegmentKey::new(0, 1))
    );
}

#[test]
fn mapped_cache_graph_generations_survive_sidecar_replacement_and_truncation() {
    use ast_segmenter::ModuleFormat;
    use segment_cache::{load_or_parse, sidecar_path, CacheMode, CacheStatus};

    let directory = test_tmpdir::TmpDir::new();
    let path = directory.root_join("leaf.mjs");
    let source = "export const value = 1;";
    std::fs::write(&path, source).unwrap();
    let load = || {
        load_or_parse(
            &logger::StdioLogger::new(),
            &path,
            ModuleFormat::Esm,
            CacheMode::ReadWrite,
        )
        .unwrap()
    };
    drop(load());
    let outcome = load();
    assert_eq!(outcome.status, CacheStatus::Hit);
    let parsed = outcome.record.to_parsed(source).unwrap();
    let first = SourceGraph::from_owned([OwnedSourceFileInput::from_parsed(
        path.clone(),
        parsed.clone(),
        Default::default(),
    )]);
    let second = SourceGraph::from_owned([OwnedSourceFileInput::from_parsed(
        path.clone(),
        parsed,
        Default::default(),
    )]);
    let name = first
        .file_facts(0)
        .unwrap()
        .bindings()
        .get(0)
        .unwrap()
        .name()
        .as_ptr();
    assert_eq!(
        second
            .file_facts(0)
            .unwrap()
            .bindings()
            .get(0)
            .unwrap()
            .name()
            .as_ptr(),
        name
    );
    drop(outcome);

    // Force publication of a new generation, then truncate its mutable sidecar.
    std::fs::write(&path, "export const replacement = 2;").unwrap();
    drop(load());
    std::fs::write(sidecar_path(&path), []).unwrap();
    drop(first);
    second.validate_links().unwrap();
    assert_eq!(
        second
            .file_facts(0)
            .unwrap()
            .bindings()
            .get(0)
            .unwrap()
            .name(),
        "value"
    );
    assert_eq!(
        second
            .file_facts(0)
            .unwrap()
            .bindings()
            .get(0)
            .unwrap()
            .name()
            .as_ptr(),
        name
    );
    assert!(second.resolve_export(0, "value", Namespace::Value).is_ok());
    assert!(second
        .resolve_export(0, "replacement", Namespace::Value)
        .is_err());
}
