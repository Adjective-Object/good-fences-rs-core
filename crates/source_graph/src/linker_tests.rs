use crate::structure::tests::input;
use crate::*;
use ast_segmenter::ExportedSymbol;

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
